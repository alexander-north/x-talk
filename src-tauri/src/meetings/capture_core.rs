//! Platform-neutral meeting capture building blocks shared by the native
//! backends: the bounded disk writer, placement of device packets on the
//! shared recording timeline, channel/rate reduction to the journal format,
//! and timing health checks. Nothing here touches an audio API, so all of it
//! is unit-tested on every platform.

use super::capture::{PeakMeter, Track};
use crate::domain::{AppError, Result};
use std::{
    fs::File,
    io::{BufWriter, Write},
    path::Path,
};

/// Journal tracks are validated against this range on recovery and by FFmpeg.
pub const MIN_RATE: u32 = 8000;
pub const MAX_RATE: u32 = 96000;
/// Device clocks and the host clock drift apart slowly. Packets are re-aligned
/// to the shared timeline only when they deviate by more than this, so normal
/// timestamp jitter never inserts or drops single frames.
pub const RESYNC_SECONDS: f64 = 0.02;

enum DiskMessage {
    Data(Vec<u8>),
    Sync,
}
// Disk latency must never stall capture. Queue capacity bounds memory to 4 MiB
// per source; saturation fails explicitly instead of dropping samples.
pub struct DiskWriter {
    sender: Option<std::sync::mpsc::SyncSender<DiskMessage>>,
    thread: Option<std::thread::JoinHandle<Result<()>>>,
    buffer: Vec<u8>,
}
impl DiskWriter {
    pub fn new(file: File) -> Result<Self> {
        let (sender, receiver) = std::sync::mpsc::sync_channel(128);
        let thread = std::thread::Builder::new()
            .name("meeting-audio-writer".into())
            .spawn(move || {
                let mut output = BufWriter::with_capacity(256 * 1024, file);
                while let Ok(message) = receiver.recv() {
                    match message {
                        DiskMessage::Data(bytes) => output.write_all(&bytes)?,
                        DiskMessage::Sync => {
                            output.flush()?;
                            output.get_ref().sync_data()?;
                        }
                    }
                }
                output.flush()?;
                output.get_ref().sync_data()?;
                Ok(())
            })?;
        Ok(Self {
            sender: Some(sender),
            thread: Some(thread),
            buffer: Vec::with_capacity(32768),
        })
    }
    fn send(&self, message: DiskMessage) -> std::io::Result<()> {
        self.sender
            .as_ref()
            .ok_or_else(|| std::io::Error::other("Audio writer is closed"))?
            .try_send(message)
            .map_err(|_| {
                std::io::Error::other(
                    "Audio could not be saved fast enough. The partial recording is preserved.",
                )
            })
    }
    pub fn sync(&mut self) -> Result<()> {
        self.flush()?;
        self.send(DiskMessage::Sync)?;
        Ok(())
    }
    pub fn finish(&mut self) -> Result<()> {
        let flushed = self.flush();
        self.sender.take();
        if let Some(thread) = self.thread.take() {
            thread
                .join()
                .map_err(|_| AppError::input("The audio writer stopped unexpectedly."))??;
        }
        flushed?;
        Ok(())
    }
}
impl Write for DiskWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let count = bytes.len().min(32768 - self.buffer.len());
        self.buffer.extend_from_slice(&bytes[..count]);
        if self.buffer.len() == 32768 {
            self.flush()?;
        }
        Ok(count)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        if !self.buffer.is_empty() {
            let buffer = std::mem::replace(&mut self.buffer, Vec::with_capacity(32768));
            self.send(DiskMessage::Data(buffer))?;
        }
        Ok(())
    }
}
impl Drop for DiskWriter {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

/// How a device's native format maps to a journal track.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Format {
    /// Journal sample rate after integer decimation.
    pub rate: u32,
    /// Journal channel count: the device's first one or two channels.
    pub channels: u16,
    /// Source frames averaged into one journal frame (1 = native rate).
    pub factor: u32,
}
/// Journal tracks are mono or stereo at 8–96 kHz. Devices with more channels
/// keep their first two (front left/right, or inputs 1–2 of an interface).
/// Devices above 96 kHz are reduced by the smallest exact integer factor.
pub fn plan_format(rate: f64, channels: u32) -> Result<Format> {
    if channels == 0 {
        return Err(AppError::input(
            "The selected audio device has no channels to record.",
        ));
    }
    let unsupported = || {
        AppError::input(format!(
            "The selected audio device runs at an unsupported sample rate ({rate} Hz). Choose 44.1 or 48 kHz for it in Audio MIDI Setup."
        ))
    };
    if !rate.is_finite() || (rate - rate.round()).abs() > 0.01 {
        return Err(unsupported());
    }
    let native = rate.round();
    if native < MIN_RATE as f64 || native > 8.0 * MAX_RATE as f64 {
        return Err(unsupported());
    }
    let native = native as u32;
    let factor = native.div_ceil(MAX_RATE);
    if !native.is_multiple_of(factor) {
        return Err(unsupported());
    }
    Ok(Format {
        rate: native / factor,
        channels: channels.min(2) as u16,
        factor,
    })
}

/// Averages `factor` consecutive frames (a boxcar low-pass) into one,
/// carrying partial frames across packets.
pub struct Converter {
    channels: usize,
    factor: usize,
    sums: [f32; 2],
    count: usize,
}
impl Converter {
    pub fn new(format: Format) -> Self {
        Self {
            channels: format.channels as usize,
            factor: format.factor.max(1) as usize,
            sums: [0.0; 2],
            count: 0,
        }
    }
    /// `input` is interleaved in the track's channel count.
    pub fn convert(&mut self, input: &[f32], output: &mut Vec<f32>) {
        output.clear();
        if self.factor == 1 {
            output.extend_from_slice(input);
            return;
        }
        for frame in input.chunks_exact(self.channels) {
            for (sum, value) in self.sums.iter_mut().zip(frame) {
                *sum += *value;
            }
            self.count += 1;
            if self.count == self.factor {
                for sum in &mut self.sums[..self.channels] {
                    output.push(*sum / self.factor as f32);
                    *sum = 0.0;
                }
                self.count = 0;
            }
        }
    }
}

/// Detects gaps in a device's own sample counter (dropped device buffers).
#[derive(Default)]
pub struct SampleClock {
    next: Option<f64>,
}
impl SampleClock {
    /// Returns true when this packet does not continue the previous one.
    pub fn observe(&mut self, sample_time: Option<f64>, frames: u32) -> bool {
        let Some(time) = sample_time.filter(|t| t.is_finite()) else {
            self.next = None;
            return false;
        };
        let broken = self.next.is_some_and(|next| (time - next).abs() >= 1.0);
        self.next = Some(time + frames as f64);
        broken
    }
    pub fn reset(&mut self) {
        self.next = None;
    }
}

/// Compares delivered frames with elapsed host time. A device whose actual
/// rate no longer matches the rate recorded in the journal would otherwise be
/// written with the wrong speed and pitch.
pub struct RateCheck {
    rate: f64,
    origin: Option<(f64, u64)>,
    frames: u64,
    next: f64,
}
impl RateCheck {
    pub const WINDOW_SECONDS: f64 = 5.0;
    pub const TOLERANCE: f64 = 0.03;
    /// A delivery pause (an idle tap, a stalled device) restarts the window
    /// instead of reading as a slow device. A wrong rate shifts each packet by
    /// only a few percent of its own length, far below this.
    pub const GAP_SECONDS: f64 = 0.25;
    pub fn new(rate: f64) -> Self {
        Self {
            rate,
            origin: None,
            frames: 0,
            next: 0.0,
        }
    }
    /// `time` is the host time of the packet's first frame in seconds.
    pub fn observe(&mut self, time: f64, frames: u32) -> Result<()> {
        let expected = self.next;
        self.next = time + frames as f64 / self.rate;
        let Some((start, counted)) = self
            .origin
            .filter(|_| (time - expected).abs() <= Self::GAP_SECONDS)
        else {
            self.origin = Some((time, 0));
            self.frames = frames as u64;
            return Ok(());
        };
        let span = time - start;
        if span >= Self::WINDOW_SECONDS {
            let measured = (self.frames - counted) as f64 / span;
            if (measured / self.rate - 1.0).abs() > Self::TOLERANCE {
                return Err(AppError::new(
                    "meeting_capture",
                    "device",
                    format!(
                        "An audio device changed its sample rate during recording (expected {:.0} Hz, measured about {:.0} Hz). The partial recording is preserved; keep the device's rate fixed in Audio MIDI Setup while recording.",
                        self.rate, measured
                    ),
                ));
            }
            self.origin = Some((time, self.frames));
        }
        self.frames += frames as u64;
        Ok(())
    }
    /// Forget the window after frames were knowingly lost.
    pub fn reset(&mut self) {
        self.origin = None;
        self.frames = 0;
    }
}

/// Core Audio device IDs from cpal have the form `coreaudio:<device UID>`;
/// UIDs themselves may contain colons. Empty means the system default.
pub fn coreaudio_uid(selected: Option<&str>) -> Option<&str> {
    let id = selected.map(str::trim).filter(|s| !s.is_empty())?;
    let uid = id
        .strip_prefix("coreaudio:")
        .or_else(|| id.strip_prefix("CoreAudio:"))
        .unwrap_or(id);
    Some(uid).filter(|s| !s.is_empty())
}

/// UID prefix of the private aggregate devices that expose macOS system-audio
/// taps. A fresh UUID follows it, so a device switch can create the next tap
/// before the previous one is destroyed (destruction is asynchronous). These
/// devices are visible only inside xTalk and must never be offered as sources.
pub const AGGREGATE_UID_PREFIX: &str = "com.xander.xtalk.meeting-tap.";
pub fn internal_device(id: &str) -> bool {
    id.contains(AGGREGATE_UID_PREFIX)
}

/// Readable form of an OSStatus: Core Audio errors are usually four-char codes.
pub fn os_status(status: i32) -> String {
    let bytes = status.to_be_bytes();
    if bytes.iter().all(|b| b.is_ascii_graphic() || *b == b' ') {
        format!("'{}'", String::from_utf8_lossy(&bytes))
    } else {
        status.to_string()
    }
}

/// Seconds of continuously unmuted, sample-exact silence after which the
/// system track shows a permission hint. A Core Audio tap delivers zeros, not
/// an error, when system audio recording is not allowed.
pub const SILENT_SYSTEM_HINT_SECONDS: f64 = 10.0;
pub fn capture_warning(
    platform: &str,
    discontinuities: u64,
    silent_system: bool,
    silent_hint: &str,
) -> Option<String> {
    let mut parts = Vec::new();
    if discontinuities > 0 {
        parts.push(format!("{platform} reported {discontinuities} audio discontinuities. Recording continued with timing preserved, but some audio may be missing. Review the recording before relying on the transcript."));
    }
    if silent_system {
        parts.push(silent_hint.to_owned());
    }
    (!parts.is_empty()).then(|| parts.join(" "))
}

/// One journal track: places packets on the shared timeline by their capture
/// time, fills gaps with silence, enforces mute, and maintains the meters.
pub struct TrackWriter {
    pub track: Track,
    output: Option<DiskWriter>,
    /// Frames written to the timeline (or counted, for preview).
    pub frames: u64,
    meter: PeakMeter,
    analyzer: crate::spectrum::Spectrum,
    pub spectrum: [f32; crate::spectrum::BANDS],
    spectrum_at: f64,
    pub discontinuities: u64,
    was_muted: bool,
    audible_after: f64,
    idle_fill: bool,
    pass_peak: f32,
    audible_frames: u64,
    signal: bool,
}
impl TrackWriter {
    /// `idle_fill` saves silence for sources that legitimately deliver no
    /// packets while idle (render loopback), instead of a large gap on resume.
    pub fn new(track: Track, output: Option<DiskWriter>, idle_fill: bool) -> Self {
        Self {
            analyzer: crate::spectrum::Spectrum::new(track.rate, track.channels),
            track,
            output,
            frames: 0,
            meter: PeakMeter::default(),
            spectrum: [0.; crate::spectrum::BANDS],
            spectrum_at: 0.,
            discontinuities: 0,
            was_muted: false,
            audible_after: f64::NEG_INFINITY,
            idle_fill,
            pass_peak: 0.0,
            audible_frames: 0,
            signal: false,
        }
    }
    pub fn create(directory: Option<&Path>, track: Track, idle_fill: bool) -> Result<Self> {
        let output = directory
            .map(|dir| DiskWriter::new(File::create(dir.join(&track.filename))?))
            .transpose()?;
        Ok(Self::new(track, output, idle_fill))
    }
    pub fn zeros(&mut self, frames: u64) -> Result<()> {
        if let Some(output) = &mut self.output {
            let zero = [0u8; 8192];
            let mut bytes = frames * self.track.channels as u64 * 4;
            while bytes > 0 {
                let count = bytes.min(zero.len() as u64) as usize;
                output.write_all(&zero[..count])?;
                bytes -= count as u64;
            }
        }
        self.frames += frames;
        Ok(())
    }
    /// Start of one polling pass. Mute changes take effect here.
    pub fn begin(&mut self, muted: bool, elapsed: f64) {
        if self.was_muted && !muted {
            // Discard packets that began before unmute: they can contain samples
            // captured while muted.
            self.audible_after = elapsed;
        }
        if muted {
            self.spectrum = [0.; crate::spectrum::BANDS];
            if !self.was_muted {
                self.analyzer =
                    crate::spectrum::Spectrum::new(self.track.rate, self.track.channels);
            }
        }
        self.was_muted = muted;
        self.pass_peak = 0.0;
    }
    /// Place one packet whose first frame was captured `start` seconds after
    /// the recording epoch. `samples` are interleaved in the track format.
    pub fn packet(&mut self, start: f64, samples: &[f32], muted: bool, elapsed: f64) -> Result<()> {
        if !start.is_finite() {
            return Err(AppError::input(
                "An audio device reported invalid timing. The partial recording is preserved.",
            ));
        }
        if start > elapsed + 1.0 {
            return Err(AppError::input(
                "Audio device timing jumped ahead. The partial recording is preserved.",
            ));
        }
        let channels = self.track.channels as usize;
        let count = (samples.len() / channels) as i64;
        let rate = self.track.rate as f64;
        let tolerance = (rate * RESYNC_SECONDS) as i64;
        let position = (start * rate).round() as i64;
        let written = self.frames as i64;
        if position - written > tolerance {
            self.zeros((position - written) as u64)?;
        }
        let skip = if written - position > tolerance {
            (written - position).min(count)
        } else {
            0
        };
        let remaining = (count - skip) as u64;
        if remaining == 0 {
            return Ok(());
        }
        if muted || start < self.audible_after {
            return self.zeros(remaining);
        }
        for &value in &samples[skip as usize * channels..count as usize * channels] {
            let value = if value.is_finite() {
                value.clamp(-1.0, 1.0)
            } else {
                0.0
            };
            self.pass_peak = self.pass_peak.max(value.abs());
            self.signal |= value != 0.0;
            if let Some(bands) = self.analyzer.push(value) {
                self.spectrum = bands;
                self.spectrum_at = elapsed;
            }
            if let Some(output) = &mut self.output {
                output.write_all(&value.to_le_bytes())?;
            }
        }
        self.frames += remaining;
        self.audible_frames += remaining;
        Ok(())
    }
    /// End of one polling pass; returns the held meter level.
    pub fn end(&mut self, muted: bool, elapsed: f64) -> Result<f32> {
        let rate = self.track.rate as f64;
        if self.idle_fill && elapsed - self.frames as f64 / rate > 1.0 {
            // Leave slack for late packets rather than writing ahead of them.
            let through = ((elapsed - 0.5).max(0.0) * rate) as u64;
            self.zeros(through.saturating_sub(self.frames))?;
        }
        if elapsed - self.spectrum_at > 0.1 {
            self.spectrum.fill(0.);
        }
        Ok(self.meter.update(self.pass_peak, elapsed, muted))
    }
    /// True once enough unmuted audio was received to judge it, all of which
    /// was exact digital silence.
    pub fn silent(&self) -> bool {
        !self.signal
            && self.audible_frames as f64 / self.track.rate as f64 >= SILENT_SYSTEM_HINT_SECONDS
    }
    pub fn sync(&mut self) -> Result<()> {
        if let Some(output) = &mut self.output {
            output.sync()?;
        }
        Ok(())
    }
    /// Close the track at exactly `elapsed` seconds. With `keep_timeline`,
    /// pad a short track with silence and trim packets that extend past Stop,
    /// keeping all tracks the same duration. After a failure only close the
    /// writer, preserving what was captured.
    pub fn finish(&mut self, elapsed: f64, directory: &Path, keep_timeline: bool) -> Result<()> {
        let target = (elapsed * self.track.rate as f64).round() as u64;
        let mut failure = None;
        if keep_timeline {
            if let Err(error) = self.zeros(target.saturating_sub(self.frames)) {
                failure = Some(error);
            }
        }
        if let Some(output) = &mut self.output {
            if let Err(error) = output.finish() {
                failure = Some(error);
            }
        }
        if failure.is_none() && keep_timeline && self.output.is_some() && self.frames > target {
            let trimmed: Result<()> = (|| {
                let file = std::fs::OpenOptions::new()
                    .write(true)
                    .open(directory.join(&self.track.filename))?;
                file.set_len(target * self.track.channels as u64 * 4)?;
                file.sync_all()?;
                Ok(())
            })();
            if let Err(error) = trimmed {
                failure = Some(error);
            } else {
                self.frames = target;
            }
        }
        failure.map_or(Ok(()), Err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn track(rate: u32, channels: u16) -> Track {
        Track {
            filename: "microphone.f32".into(),
            rate,
            channels,
            microphone: true,
        }
    }
    fn read(path: &Path) -> Vec<f32> {
        std::fs::read(path)
            .unwrap()
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect()
    }

    #[test]
    fn formats_keep_two_channels_and_decimate_high_rates_exactly() {
        let plan = |rate, channels| plan_format(rate, channels).unwrap();
        assert_eq!(
            plan(48000.0, 1),
            Format {
                rate: 48000,
                channels: 1,
                factor: 1
            }
        );
        assert_eq!(plan(44100.0, 2).rate, 44100);
        assert_eq!(plan(96000.0, 2).factor, 1);
        assert_eq!(
            plan(192000.0, 8),
            Format {
                rate: 96000,
                channels: 2,
                factor: 2
            }
        );
        assert_eq!(
            plan(176400.0, 2),
            Format {
                rate: 88200,
                channels: 2,
                factor: 2
            }
        );
        assert_eq!(
            plan(384000.0, 2),
            Format {
                rate: 96000,
                channels: 2,
                factor: 4
            }
        );
        assert_eq!(plan(8000.0, 1).rate, 8000);
        for (rate, channels) in [
            (48000.0, 0),
            (7999.0, 1),
            (f64::NAN, 2),
            (44100.5, 2),
            (97001.0, 2),
            (1_000_000.0, 2),
        ] {
            assert!(plan_format(rate, channels).is_err(), "{rate} {channels}");
        }
        for rate in [
            8000.0, 22050.0, 44100.0, 48000.0, 88200.0, 96000.0, 176400.0, 192000.0,
        ] {
            let plan = plan(rate, 2);
            assert!((MIN_RATE..=MAX_RATE).contains(&plan.rate));
        }
    }

    #[test]
    fn decimation_averages_across_packet_boundaries() {
        let format = plan_format(192000.0, 2).unwrap();
        let mut converter = Converter::new(format);
        let mut out = Vec::new();
        converter.convert(&[0.2, -0.2], &mut out);
        assert!(out.is_empty(), "a partial output frame is never emitted");
        converter.convert(&[0.4, -0.4, 1.0, 0.0], &mut out);
        assert_eq!(out.len(), 2);
        assert!((out[0] - 0.3).abs() < 1e-6 && (out[1] + 0.3).abs() < 1e-6);
        let mut native = Converter::new(plan_format(48000.0, 1).unwrap());
        native.convert(&[0.1, 0.2], &mut out);
        assert_eq!(out, vec![0.1, 0.2]);
    }

    #[test]
    fn sample_clock_flags_only_breaks_in_device_time() {
        let mut clock = SampleClock::default();
        assert!(!clock.observe(Some(1000.0), 512));
        assert!(!clock.observe(Some(1512.0), 512));
        assert!(clock.observe(Some(4096.0), 512));
        assert!(!clock.observe(None, 512));
        assert!(
            !clock.observe(Some(0.0), 512),
            "invalid time restarts tracking"
        );
        clock.reset();
        assert!(!clock.observe(Some(99.0), 1));
    }

    #[test]
    fn rate_check_accepts_drift_and_rejects_a_changed_device_rate() {
        let mut check = RateCheck::new(48000.0);
        // 48 kHz with 50 ppm drift for one minute.
        for n in 0..6000u32 {
            check
                .observe(n as f64 * 480.0 / 48002.4, 480)
                .expect("drift is accepted");
        }
        let mut changed = RateCheck::new(48000.0);
        let result = (0..1000u32).try_for_each(|n| changed.observe(n as f64 * 0.01, 441));
        let error = result.expect_err("44.1 kHz delivery is rejected");
        assert!(error.message.contains("sample rate"));
        changed.reset();
        assert!(changed.observe(100.0, 441).is_ok());
        // Delivery pauses (an idle tap) are not a rate change.
        let mut paused = RateCheck::new(48000.0);
        let mut time = 0.0;
        for n in 0..2000u32 {
            if n % 100 == 0 {
                time += 2.0;
            }
            paused
                .observe(time, 480)
                .expect("pauses restart the window");
            time += 0.01;
        }
    }

    #[test]
    fn coreaudio_ids_resolve_to_device_uids() {
        assert_eq!(coreaudio_uid(None), None);
        assert_eq!(coreaudio_uid(Some("")), None);
        assert_eq!(coreaudio_uid(Some("coreaudio:")), None);
        assert_eq!(
            coreaudio_uid(Some("coreaudio:BuiltInMicrophoneDevice")),
            Some("BuiltInMicrophoneDevice")
        );
        assert_eq!(
            coreaudio_uid(Some("coreaudio:AppleUSBAudioEngine:Vendor:Mic:1:1,2")),
            Some("AppleUSBAudioEngine:Vendor:Mic:1:1,2")
        );
        assert_eq!(coreaudio_uid(Some("RawUID")), Some("RawUID"));
        assert!(internal_device(&format!(
            "coreaudio:{AGGREGATE_UID_PREFIX}1234"
        )));
        assert!(!internal_device("coreaudio:BuiltInSpeakerDevice"));
    }

    #[test]
    fn os_status_codes_are_readable() {
        assert_eq!(os_status(0x6e6f7065), "'nope'");
        assert_eq!(os_status(0x21646576), "'!dev'");
        assert_eq!(os_status(-50), "-50");
    }

    #[test]
    fn warnings_combine_discontinuities_and_silence_hint() {
        assert_eq!(capture_warning("macOS", 0, false, "hint"), None);
        let both = capture_warning("macOS", 3, true, "Check permission.").unwrap();
        assert!(both.starts_with("macOS reported 3 audio discontinuities."));
        assert!(both.ends_with("Check permission."));
        assert_eq!(
            capture_warning("macOS", 0, true, "Check permission.").as_deref(),
            Some("Check permission.")
        );
    }

    #[test]
    fn packets_fill_gaps_skip_overlaps_and_absorb_jitter() {
        let mut writer = TrackWriter::new(track(1000, 1), None, false);
        writer.begin(false, 1.0);
        writer.packet(0.0, &[0.5; 100], false, 1.0).unwrap();
        assert_eq!(writer.frames, 100);
        // 10 ms early/late (inside the 20 ms tolerance) stays contiguous.
        writer.packet(0.09, &[0.5; 100], false, 1.0).unwrap();
        writer.packet(0.21, &[0.5; 100], false, 1.0).unwrap();
        assert_eq!(writer.frames, 300);
        // A 200 ms gap becomes silence on the timeline.
        writer.packet(0.5, &[0.5; 100], false, 1.0).unwrap();
        assert_eq!(writer.frames, 600);
        // Overlap beyond tolerance is skipped, not duplicated.
        writer.packet(0.55, &[0.5; 100], false, 1.0).unwrap();
        assert_eq!(writer.frames, 650);
        // Packets captured shortly before the epoch are trimmed to it.
        let mut early = TrackWriter::new(track(1000, 1), None, false);
        early.packet(-0.05, &[0.5; 100], false, 1.0).unwrap();
        assert_eq!(early.frames, 50);
        assert!(writer.packet(2.5, &[0.5; 10], false, 1.0).is_err());
        assert!(writer.packet(f64::NAN, &[0.5; 10], false, 1.0).is_err());
    }

    #[test]
    fn mute_writes_silence_and_drops_the_unmute_boundary_packet() {
        let dir = tempfile::tempdir().unwrap();
        let mut writer = TrackWriter::create(Some(dir.path()), track(1000, 2), false).unwrap();
        writer.begin(false, 0.1);
        writer.packet(0.0, &[0.25; 200], false, 0.1).unwrap();
        assert!(writer.end(false, 0.1).unwrap() > 0.2);
        writer.begin(true, 0.2);
        writer.packet(0.1, &[0.25; 200], true, 0.2).unwrap();
        assert_eq!(writer.end(true, 0.2).unwrap(), 0.0);
        writer.begin(false, 0.25);
        // Began before unmute: may hold muted samples.
        writer.packet(0.2, &[0.25; 200], false, 0.25).unwrap();
        writer.packet(0.3, &[0.25; 200], false, 0.4).unwrap();
        writer.finish(0.4, dir.path(), true).unwrap();
        let samples = read(&dir.path().join("microphone.f32"));
        assert_eq!(samples.len(), 800);
        assert!(samples[..200].iter().all(|v| *v == 0.25));
        assert!(samples[200..600].iter().all(|v| *v == 0.0));
        assert!(samples[600..].iter().all(|v| *v == 0.25));
    }

    #[test]
    fn finish_pads_and_trims_to_the_stop_time() {
        let dir = tempfile::tempdir().unwrap();
        let mut short = TrackWriter::create(Some(dir.path()), track(1000, 1), false).unwrap();
        short.packet(0.0, &[0.5; 300], false, 1.0).unwrap();
        short.finish(1.0, dir.path(), true).unwrap();
        let samples = read(&dir.path().join("microphone.f32"));
        assert_eq!(samples.len(), 1000);
        assert!(samples[300..].iter().all(|v| *v == 0.0));

        let mut long = TrackWriter::create(Some(dir.path()), track(1000, 1), false).unwrap();
        long.packet(0.0, &[0.5; 1200], false, 1.0).unwrap();
        long.finish(1.0, dir.path(), true).unwrap();
        assert_eq!(read(&dir.path().join("microphone.f32")).len(), 1000);

        // After a failure the captured audio is preserved untouched.
        let mut failed = TrackWriter::create(Some(dir.path()), track(1000, 1), false).unwrap();
        failed.packet(0.0, &[0.5; 1200], false, 1.0).unwrap();
        failed.finish(1.0, dir.path(), false).unwrap();
        assert_eq!(read(&dir.path().join("microphone.f32")).len(), 1200);
    }

    #[test]
    fn idle_sources_save_silence_incrementally_and_meters_decay() {
        let mut system = TrackWriter::new(track(1000, 2), None, true);
        system.begin(false, 3.0);
        assert_eq!(system.end(false, 3.0).unwrap(), 0.0);
        assert_eq!(system.frames, 2500);
        let mut microphone = TrackWriter::new(track(1000, 1), None, false);
        microphone.begin(false, 3.0);
        microphone.end(false, 3.0).unwrap();
        assert_eq!(microphone.frames, 0);
    }

    #[test]
    fn exact_digital_silence_is_reported_only_after_enough_audio() {
        let mut writer = TrackWriter::new(track(1000, 1), None, false);
        writer.packet(0.0, &vec![0.0; 9000], false, 20.0).unwrap();
        assert!(!writer.silent());
        writer.packet(9.0, &vec![0.0; 2000], false, 20.0).unwrap();
        assert!(writer.silent());
        writer.packet(11.0, &[0.0, 0.001], false, 20.0).unwrap();
        assert!(!writer.silent());
        let mut muted = TrackWriter::new(track(1000, 1), None, false);
        muted.packet(0.0, &vec![0.0; 20000], true, 20.0).unwrap();
        assert!(!muted.silent(), "muted time is not evidence");
    }

    #[test]
    fn disk_writer_persists_everything_on_finish() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.f32");
        let mut writer = DiskWriter::new(File::create(&path).unwrap()).unwrap();
        for n in 0..20_000u32 {
            writer.write_all(&(n as f32).to_le_bytes()).unwrap();
            if n % 7000 == 0 {
                writer.sync().unwrap();
            }
        }
        writer.finish().unwrap();
        let samples = read(&path);
        assert_eq!(samples.len(), 20_000);
        assert_eq!(samples[19_999], 19_999.0);
    }
}
