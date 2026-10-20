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
    pub error: Option<String>,
    pub warning: Option<String>,
}
#[derive(Clone, Deserialize)]
pub struct Options {
    pub microphone: bool,
    pub system: bool,
    pub microphone_device: Option<String>,
    pub output_device: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Track {
    pub filename: String,
    pub rate: u32,
    pub channels: u16,
    pub microphone: bool,
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
pub struct Session {
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
    pub fn finish(&self) -> Result<Journal> {
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
        output: DiskWriter,
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
            directory: &std::path::Path,
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
            let output = DiskWriter::new(File::create(directory.join(&filename))?)?;
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
                seen_packet: false,
                discontinuities: 0,
                was_muted: false,
                audible_after: 0,
            })
        }
        fn zeros(&mut self, frames: u64) -> Result<()> {
            let zero = [0u8; 8192];
            let mut bytes = frames * self.track.channels as u64 * 4;
            while bytes > 0 {
                let count = bytes.min(zero.len() as u64) as usize;
                self.output.write_all(&zero[..count])?;
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
                        self.output.write_all(&value.to_le_bytes())?;
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
            Ok(peak)
        }
        fn sync(&mut self) -> Result<()> {
            self.output.sync()
        }
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
                    &directory,
                    true,
                    options.microphone_device.as_deref(),
                )?);
            }
            if options.system {
                inputs.push(Input::open(
                    &enumerator,
                    &directory,
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
            if let Err(e) = input.output.finish() {
                failure = Some(e);
            }
        }
        let _ = crate::storage::publish_journal(&journal_path, &serde_json::to_vec(&journal)?);
        if let Ok(mut state) = status.lock() {
            state.active = false;
            state.elapsed = journal.elapsed;
            state.finalizing = true;
            state.error = failure.as_ref().map(|e| e.message.clone());
        }
        // Journal remains recoverable even if final disk sync failed.
        if let Some(error) = failure {
            return Err(error);
        }
        Ok(journal)
    }
}
