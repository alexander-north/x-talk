use crate::realtime::{LivePreview, LiveSession, StreamingProvider};
use crate::{domain::*, storage::Store};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_queue::ArrayQueue;
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Seek, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
#[derive(Clone, Serialize)]
pub struct Device {
    pub id: String,
    pub name: String,
}
#[derive(Clone, Serialize, Default)]
pub struct CaptureState {
    pub live: Option<LivePreview>,
    pub finalizing: bool,
    pub active: bool,
    pub started: Option<String>,
    pub error: Option<AppError>,
}
#[derive(Clone, Serialize, Deserialize)]
struct Journal {
    id: String,
    snapshot: Snapshot,
    state: String,
    created: String,
    sample_rate: u32,
    channels: u16,
}
pub enum Action {
    Start {
        snapshot: Snapshot,
        device: Option<String>,
    },
    Stop,
    Cancel,
    Toggle {
        snapshot: Snapshot,
        device: Option<String>,
    },
}
struct Request {
    action: Action,
    reply: std::sync::mpsc::SyncSender<Result<()>>,
}
#[derive(Clone)]
pub struct Capture {
    tx: std::sync::mpsc::Sender<Request>,
    pub state: Arc<std::sync::Mutex<CaptureState>>,
}
struct Session {
    realtime: Option<LiveSession>,
    stream: Option<cpal::Stream>,
    samples: Arc<AtomicU64>,
    max_samples: u64,
    done: Arc<AtomicBool>,
    fault: Arc<AtomicU8>,
    writer: Option<std::thread::JoinHandle<Result<u64>>>,
    journal: Journal,
    start: Instant,
}
impl Drop for Session {
    fn drop(&mut self) {
        self.stream.take();
        self.done.store(true, Ordering::Release);
        if let Some(writer) = self.writer.take() {
            let _ = writer.join();
        }
    }
}
struct Reservation {
    store: Store,
    id: String,
}
impl Drop for Reservation {
    fn drop(&mut self) {
        let _ = self.store.release(&self.id);
    }
}
pub trait CaptureFeedback: Send + Sync {
    fn changed(&self);
    fn event(&self, event: &str);
    fn queued(&self);
    fn finalizing(&self) {}
    fn spectrum(&self, _bands: [f32; crate::spectrum::BANDS]) {}
}
impl Capture {
    pub fn devices() -> Result<Vec<Device>> {
        let host = cpal::default_host();
        let mut devices = Vec::new();
        for device in host
            .input_devices()
            .map_err(|_| AppError::input("Could not enumerate microphones."))?
        {
            let id = device
                .id()
                .map_err(|_| AppError::input("Microphone identifier unavailable."))?
                .to_string();
            let name = device
                .description()
                .map(|d| device_label(&d))
                .unwrap_or_else(|_| "Microphone".into());
            devices.push(Device { id, name });
        }
        Ok(devices)
    }
    pub fn new(
        store: Store,
        feedback: Arc<dyn CaptureFeedback>,
        streaming: Arc<dyn StreamingProvider>,
    ) -> Result<Self> {
        recover(&store)?;
        let (tx, rx) = std::sync::mpsc::channel::<Request>();
        let state = Arc::new(std::sync::Mutex::new(CaptureState::default()));
        let shared = state.clone();
        let spectrum = Arc::new(std::sync::Mutex::new([0.; crate::spectrum::BANDS]));
        std::thread::Builder::new()
            .name("capture-controller".into())
            .spawn(move || {
                let mut session: Option<Session> = None;
                let mut last_spectrum = Instant::now();
                loop {
                    match if session.is_some() {
                        rx.recv_timeout(Duration::from_millis(20))
                    } else {
                        rx.recv()
                            .map_err(|_| std::sync::mpsc::RecvTimeoutError::Disconnected)
                    } {
                        Ok(req) => {
                            let result = match req.action {
                                Action::Start { snapshot, device } => {
                                    if session.is_some() {
                                        Ok(())
                                    } else {
                                        start(
                                            &store,
                                            snapshot,
                                            device,
                                            spectrum.clone(),
                                            streaming.clone(),
                                            shared.clone(),
                                            feedback.clone(),
                                        )
                                        .map(|s| {
                                            if let Ok(mut state) = shared.lock() {
                                                state.active = true;
                                                state.started = Some(now());
                                                state.error = None;
                                            }
                                            session = Some(s);
                                            feedback.event("started");
                                        })
                                    }
                                }
                                Action::Toggle { snapshot, device } => {
                                    if let Some(s) = session.take() {
                                        finish(&store, s, false, feedback.as_ref())
                                    } else {
                                        start(
                                            &store,
                                            snapshot,
                                            device,
                                            spectrum.clone(),
                                            streaming.clone(),
                                            shared.clone(),
                                            feedback.clone(),
                                        )
                                        .map(|s| {
                                            if let Ok(mut state) = shared.lock() {
                                                state.active = true;
                                                state.started = Some(now());
                                                state.error = None;
                                            }
                                            session = Some(s);
                                            feedback.event("started");
                                        })
                                    }
                                }
                                Action::Stop => {
                                    if let Some(s) = session.take() {
                                        finish(&store, s, false, feedback.as_ref())
                                    } else {
                                        Ok(())
                                    }
                                }
                                Action::Cancel => {
                                    if let Some(s) = session.take() {
                                        finish(&store, s, true, feedback.as_ref())
                                    } else {
                                        Ok(())
                                    }
                                }
                            };
                            if let Ok(mut state) = shared.lock() {
                                state.active = session.is_some();
                                if !state.active {
                                    state.started = None;
                                    state.live = None;
                                    state.finalizing = false;
                                }
                                if let Err(e) = &result {
                                    state.error = Some(e.clone());
                                }
                            }
                            if session.is_none() {
                                if let Ok(mut bands) = spectrum.lock() {
                                    *bands = [0.; crate::spectrum::BANDS];
                                }
                                feedback.spectrum([0.; crate::spectrum::BANDS]);
                                feedback.event("idle");
                            }
                            feedback.changed();
                            let _ = req.reply.send(result);
                        }
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                        Err(_) => break,
                    }
                    if session.is_some() && last_spectrum.elapsed() >= Duration::from_millis(50) {
                        if let Ok(bands) = spectrum.try_lock() {
                            feedback.spectrum(*bands);
                        }
                        last_spectrum = Instant::now();
                    }
                    if session.as_ref().is_some_and(|s| {
                        s.fault.load(Ordering::Acquire) != 0
                            || s.samples.load(Ordering::Acquire) >= s.max_samples
                    }) {
                        if let Some(s) = session.take() {
                            let result = finish(&store, s, false, feedback.as_ref());
                            if let Ok(mut state) = shared.lock() {
                                state.active = false;
                                state.started = None;
                                state.live = None;
                                state.finalizing = false;
                                state.error = result.err();
                            }
                            feedback.event("idle");
                            feedback.changed();
                        }
                    }
                }
            })?;
        Ok(Self { tx, state })
    }
    pub fn action(&self, action: Action) -> Result<()> {
        let name = match &action {
            Action::Start { .. } => "start",
            Action::Stop => "stop",
            Action::Cancel => "cancel",
            Action::Toggle { .. } => "toggle",
        };
        let started = Instant::now();
        tracing::info!(action = name, "capture_action_requested");
        let result: Result<()> = (|| {
            let (tx, rx) = std::sync::mpsc::sync_channel(1);
            self.tx
                .send(Request { action, reply: tx })
                .map_err(|_| AppError::input("Capture controller stopped."))?;
            rx.recv()
                .map_err(|_| AppError::input("Capture controller stopped."))?
        })();
        match &result {
            Ok(()) => tracing::info!(
                action = name,
                duration_ms = started.elapsed().as_millis() as u64,
                "capture_action_completed"
            ),
            Err(e) => tracing::error!(
                action = name,
                stage = e.stage,
                category = e.category,
                duration_ms = started.elapsed().as_millis() as u64,
                "capture_action_failed"
            ),
        }
        result
    }
}

fn journal_path(store: &Store, id: &str) -> PathBuf {
    store.root.join("audio").join(format!("{id}.journal"))
}
fn save_journal(store: &Store, j: &Journal) -> Result<()> {
    crate::storage::publish_journal(&journal_path(store, &j.id), &serde_json::to_vec(j)?)
}

fn start(
    store: &Store,
    snapshot: Snapshot,
    selected: Option<String>,
    spectrum: Arc<std::sync::Mutex<[f32; crate::spectrum::BANDS]>>,
    streaming: Arc<dyn StreamingProvider>,
    state: Arc<std::sync::Mutex<CaptureState>>,
    feedback: Arc<dyn CaptureFeedback>,
) -> Result<Session> {
    let realtime = snapshot.transcription_mode == TranscriptionMode::Realtime;
    if realtime {
        validate_realtime(&snapshot.transcription)?;
    }
    let host = cpal::default_host();
    let device = if let Some(selected) = selected {
        let identifier = selected.parse().map_err(|_| {
            AppError::input(
                "Selected microphone identifier is invalid. Choose the microphone again.",
            )
        })?;
        host.device_by_id(&identifier).ok_or_else(|| {
            AppError::input(
                "Selected microphone is unavailable. Choose it again or use system default.",
            )
        })?
    } else {
        host.default_input_device()
            .ok_or_else(|| AppError::input("No default microphone is available."))?
    };
    let default = device
        .default_input_config()
        .map_err(|_| AppError::input("Microphone format could not be opened."))?;
    let config = if default.channels() <= 2
        && default.sample_rate() <= 96000
        && (!realtime || crate::realtime::supported_rate(default.sample_rate()))
        && matches!(
            default.sample_format(),
            cpal::SampleFormat::F32 | cpal::SampleFormat::I16 | cpal::SampleFormat::U16
        ) {
        default
    } else {
        device
            .supported_input_configs()
            .map_err(|_| AppError::input("Microphone formats unavailable."))?
            .find(|c| {
                c.channels() <= 2
                    && c.min_sample_rate() <= 48000
                    && c.max_sample_rate() >= 48000
                    && matches!(
                        c.sample_format(),
                        cpal::SampleFormat::F32 | cpal::SampleFormat::I16 | cpal::SampleFormat::U16
                    )
            })
            .map(|c| c.with_sample_rate(48000))
            .ok_or_else(|| {
                AppError::input("Microphone needs mono/stereo PCM at 96 kHz or below.")
            })?
    };
    let id = id();
    store.reserve(&id)?;
    let result = (|| {
        if fs2::available_space(store.root.as_path())? < 128 * 1024 * 1024 {
            return Err(AppError::input(
                "At least 128 MiB free space is required to start recording.",
            ));
        }
        let journal = Journal {
            id: id.clone(),
            snapshot,
            state: "recording".into(),
            created: now(),
            sample_rate: config.sample_rate(),
            channels: config.channels(),
        };
        save_journal(store, &journal)?;
        let path = store.root.join("audio").join(format!("{id}.wav.part"));
        let spec = hound::WavSpec {
            channels: config.channels(),
            sample_rate: config.sample_rate(),
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let writer = hound::WavWriter::create(&path, spec).map_err(|_| {
            AppError::input("Could not create recording. Check available disk space.")
        })?;
        let queue = Arc::new(ArrayQueue::<f32>::new(
            (config.sample_rate() as usize) * (config.channels() as usize) * 2,
        ));
        let done = Arc::new(AtomicBool::new(false));
        let fault = Arc::new(AtomicU8::new(0));
        let samples = Arc::new(AtomicU64::new(0));
        let ready = Arc::new(AtomicBool::new(false));
        let max_samples = config.sample_rate() as u64 * config.channels() as u64 * 3600;
        let stream_config: cpal::StreamConfig = config.clone().into();
        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => build::<f32>(
                &device,
                &stream_config,
                queue.clone(),
                fault.clone(),
                samples.clone(),
                ready.clone(),
                max_samples,
            ),
            cpal::SampleFormat::I16 => build::<i16>(
                &device,
                &stream_config,
                queue.clone(),
                fault.clone(),
                samples.clone(),
                ready.clone(),
                max_samples,
            ),
            cpal::SampleFormat::U16 => build::<u16>(
                &device,
                &stream_config,
                queue.clone(),
                fault.clone(),
                samples.clone(),
                ready.clone(),
                max_samples,
            ),
            _ => unreachable!(),
        }?;
        let (live, mut audio_feed) = if realtime {
            if let Ok(mut state) = state.lock() {
                state.live = Some(LivePreview {
                    status: "connecting".into(),
                    ..Default::default()
                });
            }
            let preview = Arc::new(move |value: LivePreview| {
                if let Ok(mut state) = state.lock() {
                    state.live = Some(value);
                }
                feedback.changed();
            });
            let (live, feed) = LiveSession::start(
                streaming,
                config.sample_rate(),
                config.channels(),
                journal.snapshot.transcription.clone(),
                preview,
            )?;
            (Some(live), Some(feed))
        } else {
            (None, None)
        };
        let stop = done.clone();
        let error = fault.clone();
        let mut analyzer = crate::spectrum::Spectrum::new(config.sample_rate(), config.channels());
        let handle=std::thread::Builder::new().name("wav-writer".into()).spawn(move||{
let mut writer=writer;
let mut count=0u64;
let mut last=Instant::now();
loop{
while let Some(v)=queue.pop(){
if writer.write_sample(v).is_err(){
error.store(3,Ordering::Release);
return Err(AppError::new("capture","disk_write","Recording interrupted by a disk write error. Recoverable audio is preserved."));
}
count+=1;
if let Some(feed) = audio_feed.as_mut() { feed.push(v); }
if let Some(next) = analyzer.push(v) { if let Ok(mut bands) = spectrum.try_lock() { *bands = next; } }
}
if stop.load(Ordering::Acquire){
break;
}
if last.elapsed()>=Duration::from_secs(1){
if writer.flush().is_err(){
error.store(3,Ordering::Release);
return Err(AppError::new("capture","disk_write","Recording interrupted while checkpointing audio."));
}
last=Instant::now();
}
std::thread::sleep(Duration::from_millis(5));
}
if let Some(feed) = audio_feed.as_mut() { feed.flush(); }
drop(audio_feed);
writer.finalize().map_err(|_|AppError::new("capture","finalize","Recording could not be finalized."))?;
Ok(count)}
)?;
        let mut s = Session {
            realtime: live,
            stream: Some(stream),
            samples,
            max_samples,
            done,
            fault,
            writer: Some(handle),
            journal,
            start: Instant::now(),
        };
        if s.stream.as_ref().unwrap().play().is_err() {
            s.stream.take();
            s.done.store(true, Ordering::Release);
            if let Some(w) = s.writer.take() {
                let _ = w.join();
            }
            return Err(AppError::input(
                "Microphone could not start. Check Windows microphone privacy settings.",
            ));
        }
        let timeout = Instant::now();
        while !ready.load(Ordering::Acquire) {
            if timeout.elapsed() > Duration::from_secs(5) || s.fault.load(Ordering::Acquire) != 0 {
                s.stream.take();
                s.done.store(true, Ordering::Release);
                if let Some(w) = s.writer.take() {
                    let _ = w.join();
                }
                return Err(AppError::input("Microphone did not deliver audio. Check the selected device and Windows microphone permissions."));
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        Ok(s)
    })();
    if result.is_err() {
        let _ = store.release(&id);
        let _ = std::fs::remove_file(store.root.join("audio").join(format!("{id}.wav.part")));
        let _ = std::fs::remove_file(journal_path(store, &id));
    }
    result
}
fn build<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    queue: Arc<ArrayQueue<f32>>,
    fault: Arc<AtomicU8>,
    samples: Arc<AtomicU64>,
    ready: Arc<AtomicBool>,
    max: u64,
) -> Result<cpal::Stream>
where
    T: cpal::SizedSample,
    f32: cpal::FromSample<T>,
{
    let errors = fault.clone();
    device
        .build_input_stream(
            config,
            move |data: &[T], _| {
                use cpal::Sample;
                ready.store(true, Ordering::Release);
                let previous = samples.load(Ordering::Relaxed);
                let take = (max.saturating_sub(previous) as usize).min(data.len());
                for s in &data[..take] {
                    if queue.push(f32::from_sample(*s)).is_err() {
                        fault.store(1, Ordering::Release);
                        break;
                    }
                }
                samples.fetch_add(take as u64, Ordering::Relaxed);
            },
            move |_| {
                errors.store(2, Ordering::Release);
            },
            None,
        )
        .map_err(|_| AppError::input("Microphone input stream could not be opened."))
}
fn finish(
    store: &Store,
    mut s: Session,
    cancel: bool,
    feedback: &dyn CaptureFeedback,
) -> Result<()> {
    tracing::info!(
        capture_id = s.journal.id,
        discard = cancel,
        "capture_finish_started"
    );
    let _reservation = Reservation {
        store: store.clone(),
        id: s.journal.id.clone(),
    };
    if cancel {
        s.realtime.take();
        s.journal.state = "canceled".into();
        save_journal(store, &s.journal)?;
    }
    tracing::info!(capture_id = s.journal.id, "capture_stream_stop_started");
    drop(s.stream.take());
    let stopped = Instant::now();
    tracing::info!(capture_id = s.journal.id, "capture_stream_stop_completed");
    if !cancel {
        feedback.event("stopped");
        if s.realtime.is_some() {
            feedback.finalizing();
        }
    }
    s.done.store(true, Ordering::Release);
    tracing::info!(capture_id = s.journal.id, "capture_writer_join_started");
    let written = s
        .writer
        .take()
        .ok_or_else(|| AppError::input("Audio writer unavailable."))?
        .join()
        .map_err(|_| AppError::new("capture", "writer", "Audio writer stopped unexpectedly."))?;
    tracing::info!(capture_id = s.journal.id, "capture_writer_join_completed");
    let part = store
        .root
        .join("audio")
        .join(format!("{}.wav.part", s.journal.id));
    let relative = format!("audio/{}.wav", s.journal.id);
    let path = store.path(&relative)?;
    if cancel {
        let _ = store.release(&s.journal.id);
        if part.exists() {
            std::fs::remove_file(part)?;
        }
        std::fs::remove_file(journal_path(store, &s.journal.id))?;
        feedback.event("canceled");
        return Ok(());
    }
    let fault = s.fault.load(Ordering::Acquire);
    tracing::info!(capture_id = s.journal.id, fault, "capture_writer_result");
    if fault != 0 || written.is_err() {
        let _ = store.release(&s.journal.id);
        repair_wav(&part)?;
        std::fs::rename(&part, &path)?;
        let mut e = Entry::new(
            s.journal.id.clone(),
            "microphone",
            "Interrupted recording".into(),
            Some(relative),
            s.start.elapsed().as_secs_f64().min(MAX_SECONDS),
            path.metadata()?.len(),
        );
        e.subtitle =
            "Capture was interrupted. Review the original before retrying transcription.".into();
        store.interrupted_entry(e)?;
        std::fs::remove_file(journal_path(store, &s.journal.id))?;
        return Err(AppError::new("capture",if fault==1{
"buffer_overflow"}
else{
"interrupted"}
,"Recording interrupted by a device, buffer, or disk error. Recoverable audio is saved; retry manually."));
    }
    let count = written?;
    if count == 0 {
        store.release(&s.journal.id)?;
        let _ = std::fs::remove_file(part);
        let _ = std::fs::remove_file(journal_path(store, &s.journal.id));
        return Err(AppError::input("Recording contained no audio samples."));
    }
    tracing::info!(capture_id = s.journal.id, "capture_audio_commit_started");
    std::fs::rename(part, &path)?;
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)?
        .sync_all()?;
    tracing::info!(capture_id = s.journal.id, "capture_audio_commit_completed");
    s.journal.state = "finalized".into();
    save_journal(store, &s.journal)?;
    let duration = count as f64 / (s.journal.sample_rate as f64 * s.journal.channels as f64);
    let title = chrono::Local::now()
        .format("%b %e, %Y · %I:%M %p")
        .to_string();
    let entry = Entry::new(
        s.journal.id.clone(),
        "microphone",
        title,
        Some(relative),
        duration,
        path.metadata()?.len(),
    );
    let mut job = Job::new(
        entry.id.clone(),
        "initial",
        s.journal.snapshot.clone(),
        true,
    );
    job.realtime = s.realtime.take().map(|live| live.finish(stopped));
    // Preserve the confirmed outcome if admission is interrupted after Stop.
    if job.realtime.is_some() {
        store.stage_admission(&entry, &job)?;
    }
    tracing::info!(
        capture_id = s.journal.id,
        samples = count,
        "capture_admission_started"
    );
    store.admit(entry, job, Some(s.journal.id.clone()))?;
    store.finish_admission(&s.journal.id)?;
    tracing::info!(capture_id = s.journal.id, "capture_admission_completed");
    std::fs::remove_file(journal_path(store, &s.journal.id))?;
    feedback.queued();
    Ok(())
}
pub fn repair_wav(path: &Path) -> Result<()> {
    let mut f = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)?;
    let len = f.metadata()?.len();
    if len < 44 {
        return Err(AppError::input(
            "Interrupted recording has no recoverable audio.",
        ));
    }
    let mut header = [0u8; 12];
    f.read_exact(&mut header)?;
    if &header[..4] != b"RIFF" || &header[8..] != b"WAVE" {
        return Err(AppError::input("Interrupted recording header is invalid."));
    }
    let mut pos = 12u64;
    loop {
        if pos + 8 > len {
            return Err(AppError::input("Recording data chunk is missing."));
        }
        f.seek(std::io::SeekFrom::Start(pos))?;
        let mut chunk = [0u8; 8];
        f.read_exact(&mut chunk)?;
        let size = u32::from_le_bytes(chunk[4..8].try_into().unwrap()) as u64;
        if &chunk[..4] == b"data" {
            let bytes = (len - pos - 8) / 4 * 4;
            f.seek(std::io::SeekFrom::Start(pos + 4))?;
            f.write_all(&(bytes as u32).to_le_bytes())?;
            f.seek(std::io::SeekFrom::Start(4))?;
            f.write_all(&((len - 8) as u32).to_le_bytes())?;
            f.sync_all()?;
            return Ok(());
        }
        pos += 8 + size + (size % 2);
    }
}
fn recover_journal(store: &Store, path: &Path, identifier: &str) -> Result<()> {
    let j: Journal = serde_json::from_slice(&std::fs::read(path)?)?;
    valid_id(&j.id)?;
    if j.id != identifier
        || j.sample_rate == 0
        || !matches!(j.channels, 1 | 2)
        || !["recording", "finalized", "canceled"].contains(&j.state.as_str())
    {
        return Err(AppError::input("Recording recovery metadata is invalid."));
    }
    let part = store.root.join("audio").join(format!("{}.wav.part", j.id));
    let relative = format!("audio/{}.wav", j.id);
    let original = store.path(&relative)?;
    if store.entry(&j.id).is_ok() {
        std::fs::remove_file(path)?;
        return Ok(());
    }
    if j.state == "canceled" {
        for p in [&part, &original] {
            if p.exists() {
                std::fs::remove_file(p)?;
            }
        }
        std::fs::remove_file(path)?;
        return Ok(());
    }
    if part.exists() {
        repair_wav(&part)?;
        std::fs::rename(&part, &original)?;
    }
    if original.exists() {
        let wav = hound::WavReader::open(&original)
            .map_err(|_| AppError::input("Interrupted recording could not be recovered."))?;
        let duration = wav.duration() as f64 / wav.spec().sample_rate as f64;
        let mut e = Entry::new(
            j.id,
            "microphone",
            "Interrupted recording".into(),
            Some(relative),
            duration,
            original.metadata()?.len(),
        );
        e.created = j.created;
        e.subtitle = "Recovered after an interrupted capture. Retry transcription manually.".into();
        store.interrupted_entry(e)?;
    }
    std::fs::remove_file(path)?;
    Ok(())
}
pub fn recover(store: &Store) -> Result<()> {
    let mut paths = std::fs::read_dir(store.root.join("audio"))?
        .map(|item| item.map(|item| item.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    // An unpublished update may contain cancellation intent. Do not recover the older
    // version automatically when an interrupted update leaves that intent uncertain.
    paths.sort_by_key(|path| !path.to_string_lossy().ends_with(".journal.new"));
    for path in paths {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let Some(identifier) = name
            .strip_suffix(".journal")
            .or_else(|| name.strip_suffix(".journal.new"))
        else {
            continue;
        };
        if !path.exists() {
            continue;
        }
        if name.ends_with(".journal.new") {
            let published = path.with_extension("");
            if published.exists() {
                store.quarantine(&path);
                store.quarantine(&published);
                continue;
            }
        }
        if valid_id(identifier).is_err() || recover_journal(store, &path, identifier).is_err() {
            store.quarantine(&path);
        }
    }
    for item in std::fs::read_dir(store.root.join("tmp"))? {
        let path = item?.path();
        let result = if path.is_dir() {
            std::fs::remove_dir_all(path)
        } else {
            std::fs::remove_file(path)
        };
        if result.is_err() {
            store.warn_recovery();
        }
    }
    Ok(())
}
pub struct PlaybackControl {
    pub paused: AtomicBool,
    pub position_us: AtomicU64,
}
impl PlaybackControl {
    pub fn new(start: f64, paused: bool) -> Self {
        Self {
            paused: AtomicBool::new(paused),
            position_us: AtomicU64::new((start * 1_000_000.) as u64),
        }
    }
    pub fn position(&self) -> f64 {
        self.position_us.load(Ordering::Acquire) as f64 / 1_000_000.
    }
}
pub async fn play(
    media: &crate::media::Ffmpeg,
    path: &Path,
    stop: Arc<AtomicBool>,
    control: Arc<PlaybackControl>,
    start: f64,
) -> Result<()> {
    use tokio::io::AsyncReadExt;
    let samples = Arc::new(ArrayQueue::<f32>::new(48000 * 2 * 2));
    let finished = Arc::new(AtomicBool::new(false));
    struct Completion(Arc<AtomicBool>);
    impl Drop for Completion {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }
    let _completion = Completion(finished.clone());
    let fault = Arc::new(AtomicBool::new(false));
    let queue = samples.clone();
    let completed = finished.clone();
    let canceled = stop.clone();
    let failed = fault.clone();
    let controls = control.clone();
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let playback = tokio::task::spawn_blocking(move || {
        let result: Result<(cpal::Stream, u32, u16)> = (|| {
            let device = cpal::default_host()
                .default_output_device()
                .ok_or_else(|| AppError::input("No output device is available."))?;
            let default = device
                .default_output_config()
                .map_err(|_| AppError::input("Output device format unavailable."))?;
            let config = if default.channels() <= 2
                && default.sample_rate() <= 96000
                && matches!(
                    default.sample_format(),
                    cpal::SampleFormat::F32 | cpal::SampleFormat::I16 | cpal::SampleFormat::U16
                ) {
                default
            } else {
                device
                    .supported_output_configs()
                    .map_err(|_| AppError::input("Output device formats unavailable."))?
                    .find(|c| {
                        c.channels() <= 2
                            && c.min_sample_rate() <= 48000
                            && c.max_sample_rate() >= 48000
                            && matches!(
                                c.sample_format(),
                                cpal::SampleFormat::F32
                                    | cpal::SampleFormat::I16
                                    | cpal::SampleFormat::U16
                            )
                    })
                    .map(|c| c.with_sample_rate(48000))
                    .ok_or_else(|| {
                        AppError::input("Output device has no supported mono/stereo PCM format.")
                    })?
            };
            let stream_config: cpal::StreamConfig = config.clone().into();
            let stream = match config.sample_format() {
                cpal::SampleFormat::F32 => output::<f32>(
                    &device,
                    &stream_config,
                    queue.clone(),
                    failed.clone(),
                    controls.clone(),
                ),
                cpal::SampleFormat::I16 => output::<i16>(
                    &device,
                    &stream_config,
                    queue.clone(),
                    failed.clone(),
                    controls.clone(),
                ),
                cpal::SampleFormat::U16 => output::<u16>(
                    &device,
                    &stream_config,
                    queue.clone(),
                    failed.clone(),
                    controls.clone(),
                ),
                _ => unreachable!(),
            }?;
            stream
                .play()
                .map_err(|_| AppError::input("Output device could not start."))?;
            Ok((stream, config.sample_rate(), config.channels()))
        })();
        match result {
            Ok((stream, rate, channels)) => {
                let _ = ready_tx.send(Ok((rate, channels)));
                while !(canceled.load(Ordering::Acquire)
                    || failed.load(Ordering::Acquire)
                    || (completed.load(Ordering::Acquire) && queue.is_empty()))
                {
                    std::thread::sleep(Duration::from_millis(10));
                }
                drop(stream);
            }
            Err(e) => {
                let _ = ready_tx.send(Err(e));
            }
        }
    });
    let (rate, channels) = ready_rx
        .await
        .map_err(|_| AppError::input("Playback worker stopped."))??;
    let mut child = media
        .decode_playback_at(path, rate, channels, start)
        .await?;
    #[cfg(windows)]
    let _guard = crate::media::ChildJob::attach(&child)?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| AppError::input("Playback decoder unavailable."))?;
    let mut buffer = [0u8; 8192];
    let mut carry = Vec::<u8>::with_capacity(8196);
    let result=async{
loop{
if stop.load(Ordering::Acquire)||fault.load(Ordering::Acquire){
break;
}
let n=tokio::select!{
r=stdout.read(&mut buffer)=>r?,_=tokio::time::sleep(Duration::from_millis(50))=>continue}
;
if n==0{
break;
}
carry.extend_from_slice(&buffer[..n]);
let count=carry.len()/4*4;
for bytes in carry[..count].chunks_exact(4){
let v=f32::from_le_bytes(bytes.try_into().unwrap());
while samples.push(v).is_err(){
if stop.load(Ordering::Acquire)||fault.load(Ordering::Acquire){
break;
}
tokio::time::sleep(Duration::from_millis(2)).await;
}
if stop.load(Ordering::Acquire)||fault.load(Ordering::Acquire){
break;
}
}
carry.drain(..count);
}
if stop.load(Ordering::Acquire)||fault.load(Ordering::Acquire){
let _=child.kill().await;
}
let status=child.wait().await?;
if !status.success()&&!stop.load(Ordering::Acquire){
return Err(AppError::input("Original audio could not be played."));
}
Ok(())}
.await;
    finished.store(true, Ordering::Release);
    let _ = playback.await;
    if fault.load(Ordering::Acquire) {
        return Err(AppError::input("Playback device disconnected."));
    }
    result
}
fn output<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    queue: Arc<ArrayQueue<f32>>,
    fault: Arc<AtomicBool>,
    control: Arc<PlaybackControl>,
) -> Result<cpal::Stream>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    let samples_per_second = config.sample_rate as u64 * config.channels as u64;
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _| {
                let paused = control.paused.load(Ordering::Acquire);
                let mut consumed = 0;
                for sample in data {
                    let value = if paused { None } else { queue.pop() };
                    if value.is_some() {
                        consumed += 1;
                    }
                    *sample = T::from_sample(value.unwrap_or(0.));
                }
                control
                    .position_us
                    .fetch_add(consumed * 1_000_000 / samples_per_second, Ordering::Release);
            },
            move |_| {
                fault.store(true, Ordering::Release);
            },
            None,
        )
        .map_err(|_| AppError::input("Could not open the audio output device."))
}

/// WASAPI exposes the full Windows FriendlyName in extended metadata.
pub fn device_label(description: &cpal::DeviceDescription) -> String {
    if let Some(full) = description
        .extended()
        .iter()
        .find(|line| !line.trim().is_empty())
    {
        return full.clone();
    }
    match description
        .driver()
        .filter(|driver| *driver != description.name())
    {
        Some(driver) => format!("{} ({driver})", description.name()),
        None => description.name().to_owned(),
    }
}

#[cfg(test)]
mod recovery_tests {
    use super::*;
    fn interrupted(store: &Store, canceled: bool) -> Journal {
        let j = Journal {
            id: id(),
            snapshot: Snapshot::capture(&Settings::default(), Preset::seed()),
            state: if canceled { "canceled" } else { "recording" }.into(),
            created: now(),
            sample_rate: 24000,
            channels: 1,
        };
        let path = store.root.join("audio").join(format!("{}.wav.part", j.id));
        let mut writer = hound::WavWriter::create(
            &path,
            hound::WavSpec {
                channels: 1,
                sample_rate: 24000,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .unwrap();
        for _ in 0..2400 {
            writer.write_sample(0.05f32).unwrap();
        }
        writer.finalize().unwrap();
        // Simulate samples reaching disk after the last header checkpoint.
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        for _ in 0..2400 {
            file.write_all(&0.1f32.to_le_bytes()).unwrap();
        }
        file.sync_all().unwrap();
        save_journal(store, &j).unwrap();
        j
    }
    #[derive(Default)]
    struct RecordedFeedback(std::sync::Mutex<Vec<String>>);
    impl CaptureFeedback for RecordedFeedback {
        fn changed(&self) {}
        fn event(&self, event: &str) {
            self.0.lock().unwrap().push(event.into());
        }
        fn queued(&self) {
            self.0.lock().unwrap().push("queued".into());
        }
    }
    fn finalized_session(store: &Store, seconds: u64) -> Session {
        let rate = 1000u32;
        let count = seconds * u64::from(rate);
        let j = Journal {
            id: id(),
            snapshot: Snapshot::capture(&Settings::default(), Preset::seed()),
            state: "recording".into(),
            created: now(),
            sample_rate: rate,
            channels: 1,
        };
        store.reserve(&j.id).unwrap();
        save_journal(store, &j).unwrap();
        let path = store.root.join("audio").join(format!("{}.wav.part", j.id));
        hound::WavWriter::create(
            &path,
            hound::WavSpec {
                channels: 1,
                sample_rate: rate,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .unwrap()
        .finalize()
        .unwrap();
        let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        // Sparse, valid PCM keeps the full-hour contract cheap while retaining real file I/O.
        file.set_len(file.metadata().unwrap().len() + count * 4)
            .unwrap();
        drop(file);
        repair_wav(&path).unwrap();
        Session {
            realtime: None,
            stream: None,
            samples: Arc::new(AtomicU64::new(count)),
            max_samples: 3600 * u64::from(rate),
            done: Arc::new(AtomicBool::new(false)),
            fault: Arc::new(AtomicU8::new(0)),
            writer: Some(std::thread::spawn(move || Ok(count))),
            journal: j,
            start: Instant::now(),
        }
    }
    #[test]
    fn normal_stop_auto_cap_and_quit_share_durable_finalization_contract() {
        // Stop, auto-cap, and Stop-and-quit all call finish(cancel=false).
        for seconds in [1, 3600, 2] {
            let dir = tempfile::tempdir().unwrap();
            let store = Store::open(dir.path()).unwrap();
            let session = finalized_session(&store, seconds);
            let identifier = session.journal.id.clone();
            let feedback = RecordedFeedback::default();
            finish(&store, session, false, &feedback).unwrap();
            let entries = store.entries(0).unwrap();
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].duration, seconds as f64);
            let original = store.path(entries[0].audio.as_ref().unwrap()).unwrap();
            assert_eq!(
                hound::WavReader::open(original).unwrap().duration(),
                seconds as u32 * 1000
            );
            let jobs = store.jobs().unwrap();
            assert_eq!(jobs.len(), 1);
            assert_eq!(jobs[0].entry_id, identifier);
            assert_eq!(jobs[0].status, "queued");
            assert!(jobs[0].eligible);
            assert!(!journal_path(&store, &identifier).exists());
            assert_eq!(*feedback.0.lock().unwrap(), ["stopped", "queued"]);
        }
    }
    #[cfg(windows)]
    #[test]
    fn genuine_flush_failure_preserves_audio_and_journal_without_admission() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let session = finalized_session(&store, 1);
        let identifier = session.journal.id.clone();
        let part = store
            .root
            .join("audio")
            .join(format!("{identifier}.wav.part"));
        let original_permissions = part.metadata().unwrap().permissions();
        let mut permissions = original_permissions.clone();
        permissions.set_readonly(true);
        std::fs::set_permissions(&part, permissions).unwrap();
        assert!(finish(&store, session, false, &RecordedFeedback::default()).is_err());
        let original = store.root.join("audio").join(format!("{identifier}.wav"));
        let preserved = if original.exists() { original } else { part };
        assert!(preserved.metadata().unwrap().len() > 4000);
        assert!(journal_path(&store, &identifier).exists());
        assert!(store.entries(0).unwrap().is_empty());
        assert!(store.jobs().unwrap().is_empty());
        std::fs::set_permissions(preserved, original_permissions).unwrap();
    }
    #[test]
    fn malformed_capture_journal_does_not_block_other_recovery_or_fresh_work() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let invalid = interrupted(&store, false);
        let good = interrupted(&store, false);
        let original = store
            .root
            .join("audio")
            .join(format!("{}.wav.part", invalid.id));
        let bytes = std::fs::read(&original).unwrap();
        std::fs::write(journal_path(&store, &invalid.id), b"{partial").unwrap();
        recover(&store).unwrap();
        assert!(store.entry(&good.id).is_ok());
        assert!(store.entry(&invalid.id).is_err());
        assert_eq!(std::fs::read(original).unwrap(), bytes);
        assert!(!store.recovery_warnings().is_empty());
        let session = finalized_session(&store, 1);
        finish(&store, session, false, &RecordedFeedback::default()).unwrap();
        assert_eq!(store.entries(0).unwrap().len(), 2);
    }
    #[test]
    fn interrupted_cancellation_journal_update_never_resurrects_recording() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let j = interrupted(&store, false);
        let path = journal_path(&store, &j.id);
        std::fs::write(path.with_extension("journal.new"), b"{partial cancellation").unwrap();
        recover(&store).unwrap();
        assert!(store.entries(0).unwrap().is_empty());
        assert!(store.jobs().unwrap().is_empty());
        assert!(!store.recovery_warnings().is_empty());
        assert!(store
            .root
            .join("audio")
            .join(format!("{}.wav.part", j.id))
            .exists());
        recover(&store).unwrap();
        assert!(store.entries(0).unwrap().is_empty());
    }
    #[test]
    fn repairs_checkpointed_original_and_never_submits_it() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let j = interrupted(&store, false);
        recover(&store).unwrap();
        let e = store.entry(&j.id).unwrap();
        assert!((e.duration - 0.2).abs() < 0.0001);
        assert!(e.audio.is_some());
        assert!(store.jobs().unwrap().is_empty());
    }
    #[test]
    fn cancellation_marker_prevents_resurrection() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let j = interrupted(&store, true);
        recover(&store).unwrap();
        assert!(store.entries(0).unwrap().is_empty());
        assert!(!store
            .root
            .join("audio")
            .join(format!("{}.wav.part", j.id))
            .exists());
    }
}
