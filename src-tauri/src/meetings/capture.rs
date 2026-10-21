use crate::domain::{AppError, Result};
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

#[derive(Clone, Default, Serialize)]
pub struct Status {
    pub id: Option<String>,
    pub active: bool,
    pub finalizing: bool,
    pub elapsed: f64,
    pub microphone: bool,
    pub system: bool,
    pub microphone_muted: bool,
    pub system_muted: bool,
    pub microphone_level: f32,
    pub system_level: f32,
    pub spectrum: Vec<f32>,
    pub error: Option<String>,
    pub warning: Option<String>,
}
#[cfg_attr(not(windows), allow(dead_code))]
impl Status {
    fn capture_ended(&mut self, elapsed: f64, error: Option<&AppError>) {
        self.active = false;
        self.elapsed = elapsed;
        self.error = error.map(|e| e.message.clone());
        // Ending capture does not begin media finalization. Keep Save/Discard
        // available after a device failure or the duration limit.
    }
}
#[derive(Clone, Deserialize, PartialEq)]
pub struct Options {
    pub microphone: bool,
    pub system: bool,
    pub microphone_device: Option<String>,
    pub output_device: Option<String>,
}
/// Holds peaks across empty WASAPI passes; does not alter audio samples.
#[derive(Default)]
pub struct PeakMeter {
    value: f32,
    held_until: f64,
    updated: f64,
}
impl PeakMeter {
    pub fn update(&mut self, peak: f32, now: f64, muted: bool) -> f32 {
        if muted {
            self.value = 0.0;
            self.held_until = now;
        }
        let decay = (now - self.updated.max(self.held_until)).max(0.0);
        self.value *= 10.0_f32.powf(-24.0 * decay as f32 / 20.0);
        if !muted && peak.is_finite() && peak > 0.0 && peak >= self.value {
            self.value = peak.min(1.0);
            self.held_until = now + 0.25;
        }
        self.updated = now;
        self.value
    }
}
#[derive(Clone, Default, Serialize)]
pub struct MonitorStatus {
    pub active: bool,
    pub microphone_level: f32,
    pub system_level: f32,
    pub spectrum: Vec<f32>,
    pub error: Option<String>,
}
pub struct Monitor {
    pub options: Options,
    status: Arc<Mutex<MonitorStatus>>,
    stop: Arc<AtomicBool>,
    renewed: Arc<Mutex<std::time::Instant>>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Monitor {
    pub fn start(options: Options) -> Result<Self> {
        let status = Arc::new(Mutex::new(MonitorStatus::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let renewed = Arc::new(Mutex::new(std::time::Instant::now()));
        let (worker_status, worker_stop, worker_renewed, worker_options) = (
            status.clone(),
            stop.clone(),
            renewed.clone(),
            options.clone(),
        );
        let thread = std::thread::Builder::new()
            .name("meeting-monitor".into())
            .spawn(move || {
                #[cfg(windows)]
                let result = native::monitor(
                    worker_options,
                    worker_status.clone(),
                    worker_stop,
                    worker_renewed,
                );
                #[cfg(not(windows))]
                let result: Result<()> = {
                    let _ = (worker_options, worker_stop, worker_renewed);
                    Err(AppError::input(
                        "Device monitoring is supported on Windows.",
                    ))
                };
                if let Ok(mut state) = worker_status.lock() {
                    state.active = false;
                    state.microphone_level = 0.0;
                    state.system_level = 0.0;
                    state.spectrum.clear();
                    state.error = result.err().map(|error| {
                        format!(
                            "Could not monitor the selected devices. {}",
                            error
                                .message
                                .replace(" The partial recording is preserved.", "")
                        )
                    });
                }
            })?;
        Ok(Self {
            options,
            status,
            stop,
            renewed,
            thread: Some(thread),
        })
    }
    pub fn status(&self) -> MonitorStatus {
        self.status.lock().map(|s| s.clone()).unwrap_or_default()
    }
    pub fn renew(&self) {
        if let Ok(mut renewed) = self.renewed.lock() {
            *renewed = std::time::Instant::now();
        }
    }
    pub fn expired(&self) -> bool {
        self.thread.as_ref().is_none_or(|t| t.is_finished()) && self.status().error.is_none()
    }
}
impl Drop for Monitor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Track {
    pub filename: String,
    pub rate: u32,
    pub channels: u16,
    pub microphone: bool,
}
#[cfg(any(windows, test))]
fn compatible_format(current: &Track, next: &Track) -> Result<()> {
    if current.rate != next.rate || current.channels != next.channels {
        return Err(AppError::input("The selected device uses a different sample rate or channel count. Recording continues on the current device. Stop this meeting before choosing that device."));
    }
    Ok(())
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Journal {
    pub id: String,
    pub created: String,
    pub tracks: Vec<Track>,
    pub elapsed: f64,
    pub ready: bool,
    pub discarded: bool,
    #[serde(default)]
    pub warning: Option<String>,
}
#[cfg_attr(not(windows), allow(dead_code))]
struct DeviceChange {
    microphone: bool,
    device: Option<String>,
    reply: std::sync::mpsc::SyncSender<Result<()>>,
}
pub struct Session {
    changes: std::sync::mpsc::Sender<DeviceChange>,
    pub status: Arc<Mutex<Status>>,
    pub stop: Arc<AtomicBool>,
    pub microphone_muted: Arc<AtomicBool>,
    pub system_muted: Arc<AtomicBool>,
    thread: Mutex<Option<std::thread::JoinHandle<Result<Journal>>>>,
}
impl Session {
    pub fn start(id: String, directory: PathBuf, options: Options) -> Result<Self> {
        if !options.microphone && !options.system {
            return Err(AppError::input(
                "Enable a microphone, system audio, or both.",
            ));
        }
        let status = Arc::new(Mutex::new(Status {
            id: Some(id.clone()),
            microphone: options.microphone,
            system: options.system,
            ..Status::default()
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let microphone_muted = Arc::new(AtomicBool::new(false));
        let system_muted = Arc::new(AtomicBool::new(false));
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
        let (changes, changes_rx) = std::sync::mpsc::channel();
        let state = status.clone();
        let stop_worker = stop.clone();
        let mic = microphone_muted.clone();
        let system = system_muted.clone();
        let thread = std::thread::Builder::new()
            .name("meeting-capture".into())
            .spawn(move || {
                let result = run(
                    id,
                    directory,
                    options,
                    state.clone(),
                    stop_worker,
                    mic,
                    system,
                    &ready_tx,
                    changes_rx,
                );
                if let Err(error) = &result {
                    let _ = ready_tx.try_send(Err(error.clone()));
                    if let Ok(mut status) = state.lock() {
                        status.error = Some(error.message.clone());
                        status.active = false;
                    }
                }
                result
            })?;
        match ready_rx.recv_timeout(std::time::Duration::from_secs(15)) {
            Ok(Ok(())) => Ok(Self {
                changes,
                status,
                stop,
                microphone_muted,
                system_muted,
                thread: Mutex::new(Some(thread)),
            }),
            result => {
                stop.store(true, Ordering::Release);
                // Do not join an unresponsive driver on the application thread.
                Err(match result {
                    Ok(Err(e)) => e,
                    _ => AppError::input(
                        "Audio devices did not become ready. Check Windows audio settings.",
                    ),
                })
            }
        }
    }
    pub fn status(&self) -> Status {
        let mut status = self.status.lock().map(|s| s.clone()).unwrap_or_default();
        status.microphone_muted = self.microphone_muted.load(Ordering::Acquire);
        status.system_muted = self.system_muted.load(Ordering::Acquire);
        status
    }
    pub fn change_device(&self, microphone: bool, device: Option<String>) -> Result<()> {
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        self.changes
            .send(DeviceChange {
                microphone,
                device,
                reply,
            })
            .map_err(|_| AppError::input("Meeting capture has stopped."))?;
        response.recv().map_err(|_| {
            AppError::input("Meeting capture stopped before the device could change.")
        })?
    }
    pub fn finish(&self) -> Result<Journal> {
        self.status
            .lock()
            .map_err(|_| AppError::input("Capture unavailable."))?
            .finalizing = true;
        self.stop.store(true, Ordering::Release);
        let thread = self
            .thread
            .lock()
            .map_err(|_| AppError::input("Capture unavailable."))?
            .take()
            .ok_or_else(|| AppError::input("Meeting capture has already stopped."))?;
        thread.join().map_err(|_| {
            AppError::input(
                "Meeting capture stopped unexpectedly. Audio is preserved for recovery.",
            )
        })?
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

#[cfg(not(windows))]
#[allow(clippy::too_many_arguments)]
fn run(
    _: String,
    _: PathBuf,
    _: Options,
    _: Arc<Mutex<Status>>,
    _: Arc<AtomicBool>,
    _: Arc<AtomicBool>,
    _: Arc<AtomicBool>,
    _: &std::sync::mpsc::SyncSender<Result<()>>,
    _: std::sync::mpsc::Receiver<DeviceChange>,
) -> Result<Journal> {
    Err(AppError::input(
        "System audio recording is supported on Windows.",
    ))
}
#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
fn run(
    id: String,
    directory: PathBuf,
    options: Options,
    status: Arc<Mutex<Status>>,
    stop: Arc<AtomicBool>,
    mic_muted: Arc<AtomicBool>,
    system_muted: Arc<AtomicBool>,
    ready: &std::sync::mpsc::SyncSender<Result<()>>,
    changes: std::sync::mpsc::Receiver<DeviceChange>,
) -> Result<Journal> {
    native::record(
        id,
        directory,
        options,
        status,
        stop,
        mic_muted,
        system_muted,
        ready,
        changes,
    )
}

#[cfg(windows)]
mod native {
    use super::*;
    use std::{
        fs::File,
        io::{BufWriter, Write},
        time::{Duration, Instant},
    };
    use windows::{
        core::PCWSTR,
        Win32::{
            Media::Audio::*,
            System::{Com::*, Performance::*},
        },
    };
    fn native_error(_: windows::core::Error) -> AppError {
        AppError::new("meeting_capture", "device", "An audio device became unavailable. Captured audio is preserved; check the selected Windows devices.")
    }
    struct Com;
    impl Drop for Com {
        fn drop(&mut self) {
            unsafe {
                CoUninitialize();
            }
        }
    }
    enum DiskMessage {
        Data(Vec<u8>),
        Sync,
    }
    // Disk latency must never stall WASAPI. Queue capacity bounds memory to 4 MiB
    // per source; saturation fails explicitly instead of dropping samples.
    struct DiskWriter {
        sender: Option<std::sync::mpsc::SyncSender<DiskMessage>>,
        thread: Option<std::thread::JoinHandle<Result<()>>>,
        buffer: Vec<u8>,
    }
    impl DiskWriter {
        fn new(file: File) -> Result<Self> {
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
        fn sync(&mut self) -> Result<()> {
            self.flush()?;
            self.send(DiskMessage::Sync)?;
            Ok(())
        }
        fn finish(&mut self) -> Result<()> {
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
    struct Input {
        client: IAudioClient,
        capture: IAudioCaptureClient,
        track: Track,
        bits: u16,
        float: bool,
        frames: u64,
        output: Option<DiskWriter>,
        meter: PeakMeter,
        analyzer: crate::spectrum::Spectrum,
        spectrum: [f32; crate::spectrum::BANDS],
        spectrum_at: f64,
        seen_packet: bool,
        discontinuities: u64,
        was_muted: bool,
        audible_after: u64,
    }
    impl Drop for Input {
        fn drop(&mut self) {
            unsafe {
                let _ = self.client.Stop();
            }
        }
    }
    impl Input {
        unsafe fn open(
            enumerator: &IMMDeviceEnumerator,
            directory: Option<&std::path::Path>,
            microphone: bool,
            selected: Option<&str>,
        ) -> Result<Self> {
            let device = if let Some(id) = selected.filter(|s| !s.is_empty()) {
                let endpoint = id
                    .strip_prefix("wasapi:")
                    .or_else(|| id.strip_prefix("WASAPI:"))
                    .unwrap_or(id);
                let wide: Vec<u16> = endpoint.encode_utf16().chain(Some(0)).collect();
                enumerator.GetDevice(PCWSTR(wide.as_ptr()))
            } else {
                enumerator
                    .GetDefaultAudioEndpoint(if microphone { eCapture } else { eRender }, eConsole)
            }
            .map_err(native_error)?;
            let client: IAudioClient = device.Activate(CLSCTX_ALL, None).map_err(native_error)?;
            let format = client.GetMixFormat().map_err(native_error)?;
            let f = *format;
            let tag = if f.wFormatTag == 0xfffe && f.cbSize >= 22 {
                (*(format as *const WAVEFORMATEXTENSIBLE)).SubFormat.data1 as u16
            } else {
                f.wFormatTag
            };
            let channels = f.nChannels;
            let rate = f.nSamplesPerSec;
            let bits = f.wBitsPerSample;
            let supported = (1..=2).contains(&channels)
                && (8000..=96000).contains(&rate)
                && ((tag == 3 && f.wBitsPerSample == 32)
                    || (tag == 1 && [16, 24, 32].contains(&bits)));
            let initialized = if supported {
                client
                    .Initialize(
                        AUDCLNT_SHAREMODE_SHARED,
                        if microphone {
                            0
                        } else {
                            AUDCLNT_STREAMFLAGS_LOOPBACK
                        },
                        5_000_000,
                        0,
                        format,
                        None,
                    )
                    .map_err(native_error)
            } else {
                Err(AppError::input("Select a mono or stereo audio device with a supported Windows mix format (8–96 kHz PCM)."))
            };
            CoTaskMemFree(Some(format.cast()));
            initialized?;
            let capture: IAudioCaptureClient = client.GetService().map_err(native_error)?;
            let filename = if microphone {
                "microphone.f32"
            } else {
                "system.f32"
            }
            .to_string();
            let output = directory
                .map(|dir| DiskWriter::new(File::create(dir.join(&filename))?))
                .transpose()?;
            Ok(Self {
                client,
                capture,
                track: Track {
                    filename,
                    rate: f.nSamplesPerSec,
                    channels: f.nChannels,
                    microphone,
                },
                bits: f.wBitsPerSample,
                float: tag == 3,
                frames: 0,
                output,
                meter: PeakMeter::default(),
                analyzer: crate::spectrum::Spectrum::new(rate, channels),
                spectrum: [0.; crate::spectrum::BANDS],
                spectrum_at: 0.,
                seen_packet: false,
                discontinuities: 0,
                was_muted: false,
                audible_after: 0,
            })
        }
        fn zeros(&mut self, frames: u64) -> Result<()> {
            if self.output.is_none() {
                self.frames += frames;
                return Ok(());
            }
            let zero = [0u8; 8192];
            let mut bytes = frames * self.track.channels as u64 * 4;
            while bytes > 0 {
                let count = bytes.min(zero.len() as u64) as usize;
                self.output.as_mut().unwrap().write_all(&zero[..count])?;
                bytes -= count as u64;
            }
            self.frames += frames;
            Ok(())
        }
        unsafe fn drain(&mut self, epoch: u64, muted: bool, elapsed: f64) -> Result<f32> {
            if self.was_muted && !muted {
                // Discard the boundary packet: it can contain samples captured while muted.
                self.audible_after = epoch + (elapsed * 10_000_000.0) as u64;
            }
            if muted {
                self.spectrum = [0.; crate::spectrum::BANDS];
                if !self.was_muted {
                    self.analyzer =
                        crate::spectrum::Spectrum::new(self.track.rate, self.track.channels);
                }
            }
            self.was_muted = muted;
            let mut peak = 0.0f32;
            // Bound each pass so one device cannot starve the other.
            for _ in 0..100 {
                if self.capture.GetNextPacketSize().map_err(native_error)? == 0 {
                    break;
                }
                let mut data = std::ptr::null_mut();
                let (mut frames, mut flags, mut position, mut qpc) = (0, 0, 0, 0);
                self.capture
                    .GetBuffer(
                        &mut data,
                        &mut frames,
                        &mut flags,
                        Some(&mut position),
                        Some(&mut qpc),
                    )
                    .map_err(native_error)?;
                let result = (|| {
                    if flags & AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR.0 as u32 != 0 {
                        return Err(AppError::input("An audio device reported invalid timing. The partial recording is preserved."));
                    }
                    if flags & AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY.0 as u32 != 0
                        && self.seen_packet
                    {
                        self.discontinuities += 1;
                    }
                    if qpc.saturating_sub(epoch) as f64 / 10_000_000.0 > elapsed + 1.0 {
                        return Err(AppError::input(
                            "Audio device timing jumped ahead. The partial recording is preserved.",
                        ));
                    }
                    self.seen_packet = true;
                    let start = ((qpc.saturating_sub(epoch) as u128 * self.track.rate as u128)
                        / 10_000_000) as u64;
                    if start > self.frames {
                        self.zeros(start - self.frames)?;
                    }
                    let skip = self.frames.saturating_sub(start).min(frames as u64) as usize;
                    let count = frames as usize - skip;
                    if muted
                        || qpc < self.audible_after
                        || flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0
                    {
                        return self.zeros(count as u64);
                    }
                    if data.is_null() {
                        return Err(AppError::input("Audio device returned an empty buffer."));
                    }
                    let bytes_per_sample = self.bits as usize / 8;
                    let samples = frames as usize * self.track.channels as usize;
                    let bytes = std::slice::from_raw_parts(data, samples * bytes_per_sample);
                    for raw in bytes[skip * self.track.channels as usize * bytes_per_sample..]
                        .chunks_exact(bytes_per_sample)
                    {
                        let value = match (self.float, self.bits) {
                            (true, 32) => f32::from_le_bytes(raw.try_into().unwrap()),
                            (false, 16) => {
                                i16::from_le_bytes(raw.try_into().unwrap()) as f32 / 32768.0
                            }
                            (false, 24) => {
                                ((raw[0] as i32 | (raw[1] as i32) << 8 | (raw[2] as i32) << 16)
                                    << 8
                                    >> 8) as f32
                                    / 8388608.0
                            }
                            (false, 32) => {
                                i32::from_le_bytes(raw.try_into().unwrap()) as f32 / 2147483648.0
                            }
                            _ => unreachable!(),
                        };
                        let value = if value.is_finite() {
                            value.clamp(-1.0, 1.0)
                        } else {
                            0.0
                        };
                        peak = peak.max(value.abs());
                        if let Some(bands) = self.analyzer.push(value) {
                            self.spectrum = bands;
                            self.spectrum_at = elapsed;
                        }
                        if let Some(output) = &mut self.output {
                            output.write_all(&value.to_le_bytes())?;
                        }
                    }
                    self.frames += count as u64;
                    Ok(())
                })();
                let released = self.capture.ReleaseBuffer(frames).map_err(native_error);
                result?;
                released?;
            }
            // An idle render endpoint can provide no packets at all. Save its
            // silence incrementally, leaving a full device buffer of slack for
            // late packets, rather than allocating hours of silence on resume.
            if !self.track.microphone && elapsed - self.frames as f64 / self.track.rate as f64 > 1.0
            {
                let through = ((elapsed - 0.5).max(0.0) * self.track.rate as f64) as u64;
                self.zeros(through.saturating_sub(self.frames))?;
            }
            if elapsed - self.spectrum_at > 0.1 {
                self.spectrum.fill(0.);
            }
            Ok(self.meter.update(peak, elapsed, muted))
        }
        fn sync(&mut self) -> Result<()> {
            if let Some(output) = &mut self.output {
                output.sync()?;
            }
            Ok(())
        }
    }
    pub fn monitor(
        options: Options,
        status: Arc<Mutex<MonitorStatus>>,
        stop: Arc<AtomicBool>,
        renewed: Arc<Mutex<Instant>>,
    ) -> Result<()> {
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED)
                .ok()
                .map_err(native_error)?;
        }
        let _com = Com;
        let enumerator: IMMDeviceEnumerator = unsafe {
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(native_error)?
        };
        let mut inputs = Vec::new();
        unsafe {
            if options.microphone {
                inputs.push(Input::open(
                    &enumerator,
                    None,
                    true,
                    options.microphone_device.as_deref(),
                )?);
            }
            if options.system {
                inputs.push(Input::open(
                    &enumerator,
                    None,
                    false,
                    options.output_device.as_deref(),
                )?);
            }
        }
        let (mut counter, mut frequency) = (0, 0);
        unsafe {
            QueryPerformanceCounter(&mut counter).map_err(native_error)?;
            QueryPerformanceFrequency(&mut frequency).map_err(native_error)?;
        }
        let epoch = ((counter as u128 * 10_000_000) / frequency as u128) as u64;
        let clock = Instant::now();
        for input in &inputs {
            unsafe {
                input.client.Start().map_err(native_error)?;
            }
        }
        while !stop.load(Ordering::Acquire)
            && renewed
                .lock()
                .is_ok_and(|r| r.elapsed() < Duration::from_secs(2))
        {
            let mut levels = MonitorStatus {
                active: true,
                ..MonitorStatus::default()
            };
            for input in &mut inputs {
                let level = unsafe { input.drain(epoch, false, clock.elapsed().as_secs_f64())? };
                if input.track.microphone {
                    levels.microphone_level = level;
                } else {
                    levels.system_level = level;
                }
            }
            levels.spectrum =
                crate::spectrum::combine(inputs.iter().map(|input| &input.spectrum)).to_vec();
            if let Ok(mut state) = status.lock() {
                *state = levels;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    pub fn record(
        id: String,
        directory: PathBuf,
        options: Options,
        status: Arc<Mutex<Status>>,
        stop: Arc<AtomicBool>,
        mic_muted: Arc<AtomicBool>,
        system_muted: Arc<AtomicBool>,
        ready: &std::sync::mpsc::SyncSender<Result<()>>,
        changes: std::sync::mpsc::Receiver<super::DeviceChange>,
    ) -> Result<Journal> {
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED)
                .ok()
                .map_err(native_error)?;
        }
        let _com = Com;
        std::fs::create_dir_all(&directory)?;
        // Fail before capture when there is not even one minute of working headroom.
        if fs2::available_space(&directory)? < 128 * 1024 * 1024 {
            return Err(AppError::input(
                "Free some disk space before recording a meeting.",
            ));
        }
        let enumerator: IMMDeviceEnumerator = unsafe {
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(native_error)?
        };
        let mut inputs = Vec::new();
        unsafe {
            if options.microphone {
                inputs.push(Input::open(
                    &enumerator,
                    Some(&directory),
                    true,
                    options.microphone_device.as_deref(),
                )?);
            }
            if options.system {
                inputs.push(Input::open(
                    &enumerator,
                    Some(&directory),
                    false,
                    options.output_device.as_deref(),
                )?);
            }
        }
        let mut counter = 0;
        let mut frequency = 0;
        unsafe {
            QueryPerformanceCounter(&mut counter).map_err(native_error)?;
            QueryPerformanceFrequency(&mut frequency).map_err(native_error)?;
        }
        let epoch = ((counter as u128 * 10_000_000) / frequency as u128) as u64;
        let clock = Instant::now();
        let mut journal = Journal {
            id,
            created: crate::domain::now(),
            tracks: inputs.iter().map(|i| i.track.clone()).collect(),
            elapsed: 0.0,
            ready: false,
            discarded: false,
            warning: None,
        };
        let journal_path = directory.join("capture.json");
        crate::storage::publish_journal(&journal_path, &serde_json::to_vec(&journal)?)?;
        for input in &inputs {
            unsafe {
                input.client.Start().map_err(native_error)?;
            }
        }
        journal.ready = true;
        crate::storage::publish_journal(&journal_path, &serde_json::to_vec(&journal)?)?;
        if let Ok(mut state) = status.lock() {
            state.active = true;
        }
        let _ = ready.send(Ok(()));
        let mut last_sync = Instant::now();
        let mut failure = None;
        while !stop.load(Ordering::Acquire)
            && clock.elapsed().as_secs_f64() < super::super::domain::MAX_SECONDS
        {
            while let Ok(change) = changes.try_recv() {
                let result = (|| -> Result<()> {
                    let old = inputs
                        .iter_mut()
                        .find(|input| input.track.microphone == change.microphone)
                        .ok_or_else(|| {
                            AppError::input("This source was not enabled when recording started.")
                        })?;
                    // Open without a writer: a failed change cannot truncate the saved track.
                    let mut next = unsafe {
                        Input::open(
                            &enumerator,
                            None,
                            change.microphone,
                            change.device.as_deref(),
                        )?
                    };
                    super::compatible_format(&old.track, &next.track)?;
                    unsafe {
                        next.client.Start().map_err(native_error)?;
                    }
                    let muted = if change.microphone {
                        mic_muted.load(Ordering::Acquire)
                    } else {
                        system_muted.load(Ordering::Acquire)
                    };
                    unsafe {
                        old.drain(epoch, muted, clock.elapsed().as_secs_f64())?;
                    }
                    next.frames = old.frames;
                    next.output = old.output.take();
                    next.discontinuities = old.discontinuities;
                    next.was_muted = muted;
                    next.audible_after = old.audible_after;
                    // QPC packet timestamps preserve the common timeline, filling a switch gap
                    // with silence and skipping any overlapping frames from the new endpoint.
                    *old = next;
                    Ok(())
                })();
                let _ = change.reply.send(result);
            }
            let result: Result<()> = (|| {
                for input in &mut inputs {
                    let muted = if input.track.microphone {
                        mic_muted.load(Ordering::Acquire)
                    } else {
                        system_muted.load(Ordering::Acquire)
                    };
                    let level =
                        unsafe { input.drain(epoch, muted, clock.elapsed().as_secs_f64())? };
                    if let Ok(mut state) = status.lock() {
                        state.elapsed = clock.elapsed().as_secs_f64();
                        if input.track.microphone {
                            state.microphone_level = level;
                        } else {
                            state.system_level = level;
                        }
                    }
                }
                if let Ok(mut state) = status.lock() {
                    state.spectrum =
                        crate::spectrum::combine(inputs.iter().map(|input| &input.spectrum))
                            .to_vec();
                }
                let glitches: u64 = inputs.iter().map(|input| input.discontinuities).sum();
                if glitches > 0 {
                    journal.warning = Some(format!("Windows reported {glitches} audio discontinuities. Recording continued with timing preserved, but some audio may be missing. Review the recording before relying on the transcript."));
                    if let Ok(mut state) = status.lock() {
                        state.warning = journal.warning.clone();
                    }
                }
                if last_sync.elapsed() >= Duration::from_secs(1) {
                    for input in &mut inputs {
                        input.sync()?;
                    }
                    journal.elapsed = clock.elapsed().as_secs_f64();
                    crate::storage::publish_journal(&journal_path, &serde_json::to_vec(&journal)?)?;
                    if fs2::available_space(&directory)? < 32 * 1024 * 1024 {
                        return Err(AppError::input(
                            "Disk space is low. The partial meeting has been saved.",
                        ));
                    }
                    last_sync = Instant::now();
                }
                Ok(())
            })();
            if let Err(error) = result {
                failure = Some(error);
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        journal.elapsed = clock
            .elapsed()
            .as_secs_f64()
            .min(super::super::domain::MAX_SECONDS);
        for input in &mut inputs {
            if failure.is_none() {
                let muted = if input.track.microphone {
                    mic_muted.load(Ordering::Acquire)
                } else {
                    system_muted.load(Ordering::Acquire)
                };
                if let Err(e) = unsafe { input.drain(epoch, muted, clock.elapsed().as_secs_f64()) }
                {
                    failure = Some(e);
                }
            }
            unsafe {
                let _ = input.client.Stop();
            }
            let target = (journal.elapsed * input.track.rate as f64).round() as u64;
            if failure.is_none() {
                if let Err(e) = input.zeros(target.saturating_sub(input.frames)) {
                    failure = Some(e);
                }
            }
            if let Some(output) = &mut input.output {
                if let Err(e) = output.finish() {
                    failure = Some(e);
                }
            }
            // Endpoint packets can extend beyond the Stop instant. After the
            // writer closes, bound both source tracks to the same native clock.
            if failure.is_none() && input.frames > target {
                let trimmed: Result<()> = (|| {
                    let file = std::fs::OpenOptions::new()
                        .write(true)
                        .open(directory.join(&input.track.filename))?;
                    file.set_len(target * input.track.channels as u64 * 4)?;
                    file.sync_all()?;
                    Ok(())
                })();
                if let Err(error) = trimmed {
                    failure = Some(error);
                }
            }
        }
        let _ = crate::storage::publish_journal(&journal_path, &serde_json::to_vec(&journal)?);
        if let Ok(mut state) = status.lock() {
            state.capture_ended(journal.elapsed, failure.as_ref());
        }
        // Journal remains recoverable even if final disk sync failed.
        if let Some(error) = failure {
            return Err(error);
        }
        Ok(journal)
    }
}

#[cfg(test)]
mod meter_tests {
    use super::PeakMeter;
    #[test]
    fn automatic_capture_end_keeps_save_and_discard_available() {
        for error in [
            None,
            Some(crate::domain::AppError::input("Device disconnected")),
        ] {
            let mut status = super::Status {
                active: true,
                ..Default::default()
            };
            status.capture_ended(42.0, error.as_ref());
            assert!(!status.active);
            assert!(!status.finalizing);
            assert_eq!(status.elapsed, 42.0);
            assert_eq!(status.error.is_some(), error.is_some());
            status.finalizing = true;
            status.capture_ended(42.0, error.as_ref());
            assert!(
                status.finalizing,
                "An explicit Stop must remain busy during saving"
            );
        }
    }
    #[test]
    fn device_changes_preserve_track_format() {
        let track = super::Track {
            filename: "test.f32".into(),
            rate: 48000,
            channels: 2,
            microphone: true,
        };
        assert!(super::compatible_format(&track, &track).is_ok());
        let mut different = track.clone();
        different.rate = 44100;
        assert!(super::compatible_format(&track, &different).is_err());
        different.rate = track.rate;
        different.channels = 1;
        assert!(super::compatible_format(&track, &different).is_err());
    }
    #[test]
    fn peak_survives_empty_packets_then_decays_and_mute_clears_it() {
        let mut meter = PeakMeter::default();
        assert_eq!(meter.update(0.1, 0.0, false), 0.1);
        assert_eq!(meter.update(0.0, 0.2, false), 0.1);
        let after = meter.update(0.0, 0.75, false);
        assert!((after - 0.025118865).abs() < 0.00001);
        assert_eq!(meter.update(0.0, 0.76, true), 0.0);
        assert_eq!(meter.update(f32::NAN, 0.77, false), 0.0);
    }
}
