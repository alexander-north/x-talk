//! Bounded, interruptible event-cue playback. Capture owns priority over feedback.
use crate::{domain::*, storage::Store};
use std::{
    collections::VecDeque,
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone)]
struct Cue {
    event: String,
    preview: bool,
    queued: Instant,
}
#[derive(Default)]
struct Schedule {
    epoch: u64,
    superseded_through: i64,
    recording: bool,
    closed: bool,
    pending: VecDeque<Cue>,
    until: Option<Instant>,
}
impl Schedule {
    fn context(&self) -> Option<u64> {
        (!self.recording && !self.closed).then_some(self.epoch)
    }
    fn job_context(&self, seq: i64) -> Option<u64> {
        if seq <= self.superseded_through {
            None
        } else {
            self.context()
        }
    }
    fn interrupt(&mut self) {
        self.pending.clear();
        self.until = None;
    }
    fn begin_capture(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        self.recording = true;
        self.interrupt();
    }
    fn enqueue(&mut self, event: &str, preview: bool, context: Option<u64>, now: Instant) -> bool {
        if self.closed {
            return false;
        }
        if preview {
            if self.recording {
                return false;
            }
            self.interrupt();
        } else if matches!(event, "started" | "stopped" | "canceled") {
            self.recording = event == "started";
            self.interrupt();
        } else if self.recording || context != Some(self.epoch) {
            return false;
        }
        // Prefer dropping excess feedback over playing a growing backlog.
        if self.pending.len() >= 4 {
            return false;
        }
        self.pending.push_back(Cue {
            event: event.into(),
            preview,
            queued: now,
        });
        true
    }
    fn next(&mut self, now: Instant) -> Option<Cue> {
        if self.closed || self.until.is_some_and(|end| now < end) {
            return None;
        }
        self.until = None;
        while let Some(cue) = self.pending.pop_front() {
            if now.duration_since(cue.queued) <= Duration::from_millis(1500) {
                return Some(cue);
            }
        }
        None
    }
}

pub struct CuePlayer {
    store: Store,
    shared: Arc<(Mutex<Schedule>, Condvar)>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl CuePlayer {
    pub fn new(store: Store) -> Result<Self> {
        let shared = Arc::new((Mutex::new(Schedule::default()), Condvar::new()));
        let thread_shared = shared.clone();
        let thread_store = store.clone();
        let worker = std::thread::Builder::new()
            .name("event-cues".into())
            .spawn(move || {
                let (lock, wake) = &*thread_shared;
                let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
                loop {
                    if state.closed {
                        break;
                    }
                    let now = Instant::now();
                    if let Some(cue) = state.next(now) {
                        // Recheck at playback, so a disabled queued cue stays silent.
                        let enabled = thread_store
                            .settings()
                            .is_ok_and(|s| enabled(&s.sounds, &cue.event));
                        if cue.preview || enabled {
                            if play(&cue.event) {
                                tracing::info!(
                                    event = cue.event,
                                    preview = cue.preview,
                                    sound_set = crate::sounds::VERSION,
                                    "cue_started"
                                );
                                state.until = Some(
                                    Instant::now()
                                        + duration(&cue.event)
                                        + Duration::from_millis(20),
                                );
                            } else {
                                tracing::warn!(event = cue.event, "cue_playback_failed");
                            }
                        }
                        continue;
                    }
                    state = if let Some(end) = state.until {
                        wake.wait_timeout(state, end.saturating_duration_since(Instant::now()))
                            .unwrap_or_else(|e| e.into_inner())
                            .0
                    } else {
                        wake.wait(state).unwrap_or_else(|e| e.into_inner())
                    };
                }
            })?;
        Ok(Self {
            store,
            shared,
            worker: Some(worker),
        })
    }
    pub fn context(&self) -> Option<u64> {
        self.shared
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .context()
    }
    pub fn job_context(&self, seq: i64) -> Option<u64> {
        let state = self.shared.0.lock().unwrap_or_else(|e| e.into_inner());
        state.job_context(seq)
    }
    /// Stop app cues before opening capture, without dropping any microphone samples.
    pub fn begin_capture(&self) {
        let cutoff = self.store.last_sequence().unwrap_or(i64::MAX);
        let mut state = self.shared.0.lock().unwrap_or_else(|e| e.into_inner());
        state.begin_capture();
        state.superseded_through = cutoff;
        stop();
        self.shared.1.notify_one();
    }
    pub fn idle(&self) {
        let mut state = self.shared.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.recording {
            state.recording = false;
            state.interrupt();
            stop();
            self.shared.1.notify_one();
        }
    }
    pub fn submit(&self, event: &str, preview: bool, context: Option<u64>) -> bool {
        let mut state = self.shared.0.lock().unwrap_or_else(|e| e.into_inner());
        let priority = preview || matches!(event, "started" | "stopped" | "canceled");
        let accepted = state.enqueue(event, preview, context, Instant::now());
        if accepted
            && !preview
            && !self
                .store
                .settings()
                .is_ok_and(|s| enabled(&s.sounds, event))
        {
            state.pending.pop_back();
        }
        if accepted {
            if priority {
                stop();
            }
            self.shared.1.notify_one();
        } else {
            tracing::info!(event, preview, "cue_suppressed");
        }
        accepted
    }
    pub fn shutdown(&self) {
        let mut state = self.shared.0.lock().unwrap_or_else(|e| e.into_inner());
        state.closed = true;
        state.interrupt();
        stop();
        self.shared.1.notify_one();
    }
}
impl Drop for CuePlayer {
    fn drop(&mut self) {
        self.shutdown();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
fn enabled(s: &Sounds, event: &str) -> bool {
    match event {
        "started" => s.started,
        "stopped" => s.stopped,
        "canceled" => s.canceled,
        "transcribed" => s.transcribed,
        "transformed" => s.transformed,
        "failed" => s.failed,
        _ => false,
    }
}
fn duration(event: &str) -> Duration {
    Duration::from_millis(match event {
        "started" => 400,
        "stopped" => 330,
        "canceled" => 320,
        "transcribed" => 440,
        "transformed" => 350,
        "failed" => 300,
        _ => 0,
    })
}
#[cfg(windows)]
fn play(event: &str) -> bool {
    let Ok(bytes) = crate::sounds::asset(event) else {
        return false;
    };
    // Embedded bytes have static lifetime, including throughout asynchronous playback.
    unsafe {
        windows_sys::Win32::Media::Audio::PlaySoundW(
            bytes.as_ptr().cast(),
            std::ptr::null_mut(),
            windows_sys::Win32::Media::Audio::SND_MEMORY
                | windows_sys::Win32::Media::Audio::SND_ASYNC
                | windows_sys::Win32::Media::Audio::SND_NODEFAULT,
        ) != 0
    }
}
#[cfg(windows)]
fn stop() {
    unsafe {
        windows_sys::Win32::Media::Audio::PlaySoundW(std::ptr::null(), std::ptr::null_mut(), 0);
    }
}
#[cfg(all(feature = "desktop", not(windows)))]
fn play(event: &str) -> bool {
    crate::sounds::asset(event).is_ok_and(output::play)
}
#[cfg(all(feature = "desktop", not(windows)))]
fn stop() {
    output::stop();
}
#[cfg(not(any(windows, feature = "desktop")))]
fn play(_: &str) -> bool {
    false
}
#[cfg(not(any(windows, feature = "desktop")))]
fn stop() {}

/// Decode an embedded 16-bit mono cue and resample it linearly to the output device rate.
/// Linear interpolation is adequate for these short synthesized cues.
#[cfg(any(test, all(feature = "desktop", not(windows))))]
fn render(wav: &[u8], rate: u32) -> Option<Vec<f32>> {
    let reader = hound::WavReader::new(std::io::Cursor::new(wav)).ok()?;
    let spec = reader.spec();
    if spec.channels != 1
        || spec.bits_per_sample != 16
        || spec.sample_format != hound::SampleFormat::Int
        || spec.sample_rate == 0
        || rate == 0
    {
        return None;
    }
    let source: Vec<f32> = reader
        .into_samples::<i16>()
        .map(|s| s.map(|v| f32::from(v) / 32768.))
        .collect::<std::result::Result<_, _>>()
        .ok()?;
    if spec.sample_rate == rate || source.is_empty() {
        return Some(source);
    }
    let step = f64::from(spec.sample_rate) / f64::from(rate);
    let frames = (source.len() as f64 / step).round() as usize;
    Some(
        (0..frames)
            .map(|i| {
                let at = i as f64 * step;
                let index = at.floor() as usize;
                let fraction = (at - index as f64) as f32;
                let a = source.get(index).copied().unwrap_or(0.);
                let b = source.get(index + 1).copied().unwrap_or(0.);
                a + (b - a) * fraction
            })
            .collect(),
    )
}

/// Asynchronous cue output through cpal (CoreAudio on macOS), mirroring the Windows
/// `PlaySoundW(SND_ASYNC | SND_MEMORY)` contract: `play` returns at once, a new cue
/// replaces the one still sounding, and `stop` silences it. One thread owns the stream,
/// and the output device is open only while a cue sounds.
#[cfg(all(feature = "desktop", not(windows)))]
mod output {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, OnceLock,
    };
    use std::time::Duration;

    enum Command {
        Play(&'static [u8]),
        Stop,
    }
    fn sender() -> Option<&'static mpsc::Sender<Command>> {
        static OUTPUT: OnceLock<Option<mpsc::Sender<Command>>> = OnceLock::new();
        OUTPUT
            .get_or_init(|| {
                let (tx, rx) = mpsc::channel();
                std::thread::Builder::new()
                    .name("cue-output".into())
                    .spawn(move || run(rx))
                    .ok()
                    .map(|_| tx)
            })
            .as_ref()
    }
    pub(super) fn play(wav: &'static [u8]) -> bool {
        sender().is_some_and(|tx| tx.send(Command::Play(wav)).is_ok())
    }
    pub(super) fn stop() {
        if let Some(tx) = sender() {
            let _ = tx.send(Command::Stop);
        }
    }
    fn run(rx: mpsc::Receiver<Command>) {
        let mut current: Option<(cpal::Stream, Arc<AtomicBool>)> = None;
        loop {
            let command = if current.is_some() {
                match rx.recv_timeout(Duration::from_millis(50)) {
                    Ok(command) => Some(command),
                    Err(mpsc::RecvTimeoutError::Timeout) => None,
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            } else {
                match rx.recv() {
                    Ok(command) => Some(command),
                    Err(_) => break,
                }
            };
            match command {
                Some(Command::Stop) => current = None,
                Some(Command::Play(wav)) => {
                    // Interrupt first, so a failed open never leaves the previous cue sounding.
                    current = None;
                    match open(wav) {
                        Ok(stream) => current = Some(stream),
                        Err(reason) => tracing::warn!(reason, "cue_output_failed"),
                    }
                }
                None => {}
            }
            if current
                .as_ref()
                .is_some_and(|(_, done)| done.load(Ordering::Acquire))
            {
                current = None;
            }
        }
    }
    fn open(wav: &'static [u8]) -> Result<(cpal::Stream, Arc<AtomicBool>), &'static str> {
        let device = cpal::default_host()
            .default_output_device()
            .ok_or("no_output_device")?;
        // Use the device's current format: requesting another rate could change the
        // hardware rate for every application on macOS.
        let config = device
            .default_output_config()
            .map_err(|_| "output_format_unavailable")?;
        let samples = super::render(wav, config.sample_rate()).ok_or("cue_decode_failed")?;
        let channels = usize::from(config.channels().max(1));
        // Keep the stream open briefly after the last sample so the device drains it.
        let tail = config.sample_rate() as usize / 10;
        let done = Arc::new(AtomicBool::new(false));
        let stream_config: cpal::StreamConfig = config.clone().into();
        let voice = Voice {
            samples,
            channels,
            tail,
            done: done.clone(),
        };
        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => build::<f32>(&device, &stream_config, voice),
            cpal::SampleFormat::I16 => build::<i16>(&device, &stream_config, voice),
            cpal::SampleFormat::U16 => build::<u16>(&device, &stream_config, voice),
            cpal::SampleFormat::I32 => build::<i32>(&device, &stream_config, voice),
            _ => return Err("output_format_unsupported"),
        }?;
        stream.play().map_err(|_| "output_start_failed")?;
        Ok((stream, done))
    }
    struct Voice {
        samples: Vec<f32>,
        channels: usize,
        tail: usize,
        done: Arc<AtomicBool>,
    }
    fn build<T>(
        device: &cpal::Device,
        config: &cpal::StreamConfig,
        voice: Voice,
    ) -> Result<cpal::Stream, &'static str>
    where
        T: cpal::SizedSample + cpal::FromSample<f32>,
    {
        let Voice {
            samples,
            channels,
            tail,
            done,
        } = voice;
        let failed = done.clone();
        let end = samples.len() + tail;
        let mut position = 0usize;
        device
            .build_output_stream(
                config,
                move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                    for frame in data.chunks_mut(channels) {
                        let value = samples.get(position).copied().unwrap_or(0.);
                        position = position.saturating_add(1);
                        for sample in frame {
                            *sample = T::from_sample(value);
                        }
                    }
                    if position >= end {
                        done.store(true, Ordering::Release);
                    }
                },
                move |_| failed.store(true, Ordering::Release),
                None,
            )
            .map_err(|_| "output_open_failed")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fast_completions_wait_for_the_whole_previous_phrase() {
        let mut s = Schedule::default();
        let t = Instant::now();
        assert!(s.enqueue("transcribed", false, Some(0), t));
        assert_eq!(s.next(t).unwrap().event, "transcribed");
        s.until = Some(t + duration("transcribed"));
        assert!(s.enqueue("transformed", false, Some(0), t));
        assert!(s.next(t + Duration::from_millis(439)).is_none());
        assert_eq!(
            s.next(t + Duration::from_millis(440)).unwrap().event,
            "transformed"
        );
    }
    #[test]
    fn capture_interrupts_and_old_jobs_stay_silent_after_capture_ends() {
        let mut s = Schedule::default();
        let t = Instant::now();
        let old = s.context();
        s.enqueue("transcribed", false, old, t);
        s.begin_capture();
        assert!(s.pending.is_empty());
        assert!(s.context().is_none());
        assert!(!s.enqueue("transformed", false, old, t));
        assert!(!s.enqueue("transformed", true, None, t));
        assert!(s.enqueue("started", false, None, t));
        assert!(s.enqueue("canceled", false, None, t));
        assert!(!s.enqueue("transformed", false, old, t));
        assert!(!s.enqueue("transformed", false, None, t));
        assert_eq!(s.next(t).unwrap().event, "canceled");
        assert!(s.enqueue("transformed", false, s.context(), t));
    }
    #[test]
    fn preview_replaces_preview_and_backlogs_expire_and_are_bounded() {
        let mut s = Schedule::default();
        let t = Instant::now();
        s.enqueue("started", true, None, t);
        s.enqueue("stopped", true, None, t);
        assert_eq!(s.pending.len(), 1);
        assert_eq!(s.next(t).unwrap().event, "stopped");
        for _ in 0..4 {
            assert!(s.enqueue("transcribed", false, Some(0), t));
        }
        assert!(!s.enqueue("transcribed", false, Some(0), t));
        assert!(s.next(t + Duration::from_secs(2)).is_none());
        s.closed = true;
        assert!(!s.enqueue("started", false, None, t));
    }
    #[test]
    fn queued_old_jobs_are_silent_but_fresh_jobs_and_retries_can_notify() {
        let mut s = Schedule::default();
        s.begin_capture();
        s.superseded_through = 7;
        assert!(s.job_context(8).is_none());
        s.recording = false;
        assert!(s.job_context(7).is_none());
        assert_eq!(s.job_context(8), Some(s.epoch));
    }
    #[test]
    fn failure_cue_is_ordinary_feedback_that_never_interrupts_capture() {
        let mut s = Schedule::default();
        let t = Instant::now();
        s.begin_capture();
        assert!(!s.enqueue("failed", false, s.context(), t));
        assert!(!s.enqueue("failed", false, Some(s.epoch), t));
        assert!(
            s.recording,
            "a failure cue must not end the recording state"
        );
        assert!(s.pending.is_empty());
        s.recording = false;
        assert!(s.enqueue("transcribed", false, s.context(), t));
        assert!(s.enqueue("failed", false, s.context(), t));
        assert!(!s.recording);
        assert_eq!(s.next(t).unwrap().event, "transcribed");
        assert_eq!(s.next(t).unwrap().event, "failed");
        let sounds = Settings::default().sounds;
        assert!(enabled(&sounds, "failed"));
        assert!(!enabled(
            &Sounds {
                failed: false,
                ..sounds
            },
            "failed"
        ));
    }
    #[test]
    fn rendering_preserves_duration_and_level_at_common_device_rates() {
        for event in ["started", "failed"] {
            let wav = crate::sounds::asset(event).unwrap();
            let native = render(wav, 48000).unwrap();
            assert_eq!(
                native.len() as u64 * 1000 / 48000,
                duration(event).as_millis() as u64
            );
            let peak = |s: &[f32]| s.iter().fold(0f32, |m, v| m.max(v.abs()));
            for rate in [44100, 96000, 16000] {
                let resampled = render(wav, rate).unwrap();
                let expected = native.len() as f64 * f64::from(rate) / 48000.;
                assert!((resampled.len() as f64 - expected).abs() <= 1.);
                assert!(peak(&resampled) <= peak(&native) + f32::EPSILON);
                assert!(peak(&resampled) > peak(&native) * 0.8);
            }
        }
        assert!(render(b"not a wav", 48000).is_none());
        assert!(render(crate::sounds::asset("started").unwrap(), 0).is_none());
    }
    #[test]
    fn timing_matches_embedded_audio() {
        for event in [
            "started",
            "stopped",
            "canceled",
            "transcribed",
            "transformed",
            "failed",
        ] {
            let r =
                hound::WavReader::new(std::io::Cursor::new(crate::sounds::asset(event).unwrap()))
                    .unwrap();
            assert_eq!(
                u64::from(r.duration()) * 1000 / u64::from(r.spec().sample_rate),
                duration(event).as_millis() as u64
            );
        }
    }
}
