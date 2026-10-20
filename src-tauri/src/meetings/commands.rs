use super::{domain::*, export, media, store::MeetingStore};
use crate::{
    app::AppState,
    domain::{id, AppError, Result},
};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::atomic::Ordering};
use tauri::{Emitter, Manager};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_dialog::DialogExt;

pub struct PendingImport {
    pub token: String,
    pub path: PathBuf,
    pub inspection: media::Inspection,
}
#[derive(Serialize)]
pub struct ImportPreview {
    token: String,
    filename: String,
    inspection: media::Inspection,
}
#[derive(Serialize)]
pub struct View {
    meeting: export::MeetingExport,
    error: Option<String>,
    merges: usize,
    replacement: bool,
}
fn view(m: Meeting) -> Result<View> {
    Ok(View {
        meeting: export::project(&m)?,
        error: m.attempt.error,
        merges: m.transcript.as_ref().map_or(0, |t| t.merges.len()),
        replacement: m.attempt.replacement,
    })
}
pub fn changed(app: &tauri::AppHandle) {
    if let Some(state) = app.try_state::<AppState>() {
        let meeting = state.meeting_active.load(Ordering::Acquire);
        let dictation = state
            .capture
            .state
            .lock()
            .map(|s| s.active)
            .unwrap_or(false);
        if let Some(tray) = app.tray_by_id("main") {
            let _ = tray.set_icon(Some(crate::app::tray_image(meeting || dictation)));
            let _ = tray.set_tooltip(Some(if meeting {
                "x-talk · Recording meeting"
            } else if dictation {
                "x-talk · Recording"
            } else {
                "x-talk · Ready"
            }));
        }
    }
    let _ = app.emit("meetings-changed", ());
}
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|_| AppError::input("Meeting operation was interrupted."))?
}
#[tauri::command]
pub async fn meetings_list(app: tauri::AppHandle) -> Result<Vec<Summary>> {
    blocking(move || app.state::<AppState>().meetings.summaries()).await
}
#[tauri::command]
pub async fn meeting_get(app: tauri::AppHandle, id: String) -> Result<View> {
    blocking(move || view(app.state::<AppState>().meetings.get(&id)?)).await
}
#[tauri::command]
pub async fn meeting_pick(app: tauri::AppHandle) -> Result<Option<ImportPreview>> {
    let picker = app.clone();
    let path = blocking(move || {
        Ok(picker
            .dialog()
            .file()
            .add_filter(
                "Audio or video",
                &[
                    "wav", "mp3", "m4a", "aac", "flac", "mp4", "mov", "mkv", "webm", "ogg", "opus",
                ],
            )
            .blocking_pick_file()
            .and_then(|p| p.into_path().ok()))
    })
    .await?;
    let Some(path) = path else {
        return Ok(None);
    };
    let s = app.state::<AppState>();
    let inspection = media::inspect(&s.media, &path).await?;
    let token = id();
    let filename = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    *s.meeting_import
        .lock()
        .map_err(|_| AppError::input("Import unavailable."))? = Some(PendingImport {
        token: token.clone(),
        path,
        inspection: inspection.clone(),
    });
    Ok(Some(ImportPreview {
        token,
        filename,
        inspection,
    }))
}
#[tauri::command]
pub async fn meeting_import(app: tauri::AppHandle, token: String) -> Result<String> {
    let s = app.state::<AppState>();
    let _activity = s.worker_activity.read().await;
    if s.shutting_down.load(Ordering::Acquire) {
        return Err(AppError::input("Application is closing."));
    }
    let _import = s.meeting_import_gate.lock().await;
    let pending = {
        let mut slot = s
            .meeting_import
            .lock()
            .map_err(|_| AppError::input("Import unavailable."))?;
        if slot.as_ref().is_none_or(|p| p.token != token) {
            return Err(AppError::input("Choose the file again."));
        }
        slot.take().unwrap()
    };
    let filename = pending
        .path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let mut m = Meeting::new(
        pending
            .path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string(),
        if pending.inspection.video {
            SourceKind::Video
        } else {
            SourceKind::Audio
        },
        Some(filename),
    );
    let directory = s.meetings.root.join("audio").join(&m.id);
    tokio::fs::create_dir_all(&directory).await?;
    let pending_path = directory.join("import.pending");
    let pending_bytes = serde_json::to_vec(&m)?;
    blocking(move || crate::storage::publish_journal(&pending_path, &pending_bytes)).await?;
    let prepared = directory.join("prepared.m4a");
    let result: Result<()> = async {
        let info = media::prepare(&s.media, &pending.path, &prepared).await?;
        m.duration = Some(info.duration);
        m.audio_hash = Some(media::content_hash(&prepared).await?);
        let mut assets = vec!["prepared.m4a".to_string()];
        let download = if pending.inspection.video {
            "prepared.m4a".into()
        } else {
            let ext = pending
                .path
                .extension()
                .and_then(|s| s.to_str())
                .filter(|s| s.chars().all(|c| c.is_ascii_alphanumeric()))
                .unwrap_or("audio");
            let name = format!("original.{ext}");
            tokio::fs::copy(&pending.path, directory.join(&name)).await?;
            assets.push(name.clone());
            name
        };
        m.audio = Some(Audio {
            download,
            prepared: Some("prepared.m4a".into()),
            assets,
        });
        let store = s.meetings.clone();
        let saved = m.clone();
        blocking(move || store.insert(saved)).await?;
        Ok(())
    }
    .await;
    if result.is_err() && s.meetings.get(&m.id).is_err() {
        let _ = tokio::fs::remove_dir_all(&directory).await;
    }
    result?;
    s.meetings_wake.notify_one();
    changed(&app);
    Ok(m.id)
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Edit {
    Metadata {
        title: String,
        call_at: String,
    },
    Correct {
        turn: String,
        correction: Option<Correction>,
    },
    Name {
        speaker: String,
        name: String,
    },
    Merge {
        from: String,
        into: String,
    },
    UndoMerge,
    Retry {
        transcription: bool,
    },
}
#[tauri::command]
pub async fn meeting_edit(
    app: tauri::AppHandle,
    id: String,
    revision: u64,
    edit: Edit,
) -> Result<View> {
    let handle = app.clone();
    let m = blocking(move || {
        handle
            .state::<AppState>()
            .meetings
            .edit(&id, revision, move |m| {
                match edit {
                    Edit::Metadata { title, call_at } => {
                        if title.trim().is_empty()
                            || title.len() > 500
                            || chrono::DateTime::parse_from_rfc3339(&call_at).is_err()
                        {
                            return Err(AppError::input("Enter a title and valid meeting date."));
                        }
                        m.title = title.trim().into();
                        m.manual_title = true;
                        m.call_at = call_at;
                    }
                    Edit::Correct { turn, correction } => m.correct(&turn, correction)?,
                    Edit::Name { speaker, name } => m.name_speaker(&speaker, &name)?,
                    Edit::Merge { from, into } => m.merge(&from, &into)?,
                    Edit::UndoMerge => m.undo_merge()?,
                    Edit::Retry { transcription } => m.retry(transcription)?,
                }
                Ok(())
            })
    })
    .await?;
    app.state::<AppState>().meetings_wake.notify_one();
    changed(&app);
    view(m)
}
#[tauri::command]
pub async fn meeting_copy(app: tauri::AppHandle, id: String) -> Result<()> {
    blocking(move || {
        let m = app.state::<AppState>().meetings.get(&id)?;
        app.clipboard()
            .write_text(export::json(&m)?)
            .map_err(|_| AppError::input("The clipboard is busy. Try Copy meeting JSON again."))
    })
    .await
}
#[tauri::command]
pub async fn meeting_download(app: tauri::AppHandle, id: String) -> Result<()> {
    let state = app.state::<AppState>();
    let _activity = state.worker_activity.read().await;
    if state.shutting_down.load(Ordering::Acquire) {
        return Err(AppError::input("Application is closing."));
    }
    let handle = app.clone();
    blocking(move || {
        let store = &handle.state::<AppState>().meetings;
        let _lease = store.lease(&id)?;
        let m = store.get(&id)?;
        let audio = m
            .audio
            .ok_or_else(|| AppError::input("Meeting audio is no longer retained."))?;
        let source = store.audio_path(&id, &audio.download)?;
        let filename = format!(
            "meeting-{}.{}",
            &id[..8],
            source.extension().unwrap_or_default().to_string_lossy()
        );
        if let Some(dest) = handle
            .dialog()
            .file()
            .set_file_name(filename)
            .blocking_save_file()
            .and_then(|p| p.into_path().ok())
        {
            let parent = dest
                .parent()
                .ok_or_else(|| AppError::input("Choose a destination folder."))?
                .canonicalize()?;
            if parent.starts_with(store.root.canonicalize()?) {
                return Err(AppError::input(
                    "Save audio outside xTalk's managed storage.",
                ));
            }
            std::fs::copy(source, dest)?;
        }
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn meeting_play(app: tauri::AppHandle, id: String, start: f64) -> Result<()> {
    if !start.is_finite() || !(0.0..=MAX_SECONDS).contains(&start) {
        return Err(AppError::input("Invalid playback time."));
    }
    let state = app.state::<AppState>();
    let _controls = state.controls.lock().await;
    if state.shutting_down.load(Ordering::Acquire)
        || state.meeting_active.load(Ordering::Acquire)
        || state
            .capture
            .state
            .lock()
            .map_err(|_| AppError::input("Capture unavailable."))?
            .active
    {
        return Err(AppError::input(
            "Stop recording before playing meeting audio.",
        ));
    }
    crate::app::stop_playback(&app).await;
    let lease = state.meetings.lease(&id)?;
    let m = state.meetings.get(&id)?;
    let path = state.meetings.audio_path(
        &id,
        &m.audio
            .ok_or_else(|| AppError::input("Audio unavailable."))?
            .download,
    )?;
    let duration = m.duration.unwrap_or(0.0);
    let start = start.min(duration);
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let done = std::sync::Arc::new(tokio::sync::Notify::new());
    let control = std::sync::Arc::new(crate::capture::PlaybackControl::new(start, false));
    *state
        .playback
        .lock()
        .map_err(|_| AppError::input("Playback unavailable."))? = Some(crate::app::Playback {
        id: format!("meeting:{id}"),
        stop: stop.clone(),
        done: done.clone(),
        control: control.clone(),
        duration,
    });
    let media = state.media.clone();
    let store = state.meetings.clone();
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let result = crate::capture::play(&media, &path, stop.clone(), control, start).await;
        if let Err(error) = result {
            let _ = handle.emit("playback-error", error);
        }
        if let Ok(mut current) = handle.state::<AppState>().playback.lock() {
            if current
                .as_ref()
                .is_some_and(|p| std::sync::Arc::ptr_eq(&p.stop, &stop))
            {
                *current = None;
            }
        }
        drop(lease);
        let _ = store.prune();
        done.notify_one();
        crate::app::changed(&handle);
    });
    crate::app::changed(&app);
    Ok(())
}
#[tauri::command]
pub async fn meeting_delete(app: tauri::AppHandle, id: String, revision: u64) -> Result<()> {
    let handle = app.clone();
    blocking(move || handle.state::<AppState>().meetings.delete(&id, revision)).await?;
    changed(&app);
    Ok(())
}
#[tauri::command]
pub async fn meeting_retention(app: tauri::AppHandle) -> Result<Option<u32>> {
    blocking(move || app.state::<AppState>().meetings.retention()).await
}
#[tauri::command]
pub async fn meeting_set_retention(app: tauri::AppHandle, count: Option<u32>) -> Result<()> {
    let handle = app.clone();
    blocking(move || handle.state::<AppState>().meetings.set_retention(count)).await?;
    changed(&app);
    Ok(())
}

#[derive(Serialize)]
pub struct Devices {
    microphones: Vec<crate::capture::Device>,
    outputs: Vec<crate::capture::Device>,
}
#[tauri::command]
pub async fn meeting_devices() -> Result<Devices> {
    blocking(|| {
        use cpal::traits::{DeviceTrait, HostTrait};
        let host = cpal::default_host();
        let microphones = crate::capture::Capture::devices()?;
        let outputs = host
            .output_devices()
            .map_err(|_| AppError::input("Could not list output devices."))?
            .filter_map(|d| {
                Some(crate::capture::Device {
                    id: d.id().ok()?.to_string(),
                    name: d.description().ok()?.name().to_string(),
                })
            })
            .collect();
        Ok(Devices {
            microphones,
            outputs,
        })
    })
    .await
}
#[tauri::command]
pub fn meeting_capture_status(app: tauri::AppHandle) -> Result<super::capture::Status> {
    let state = app.state::<AppState>();
    let session = state
        .meeting_capture
        .lock()
        .map_err(|_| AppError::input("Meeting capture unavailable."))?;
    Ok(session
        .as_ref()
        .map_or_else(super::capture::Status::default, |s| s.status()))
}
#[tauri::command]
pub async fn meeting_start(app: tauri::AppHandle, options: super::capture::Options) -> Result<()> {
    let state = app.state::<AppState>();
    let _controls = state.controls.lock().await;
    if state.shutting_down.load(Ordering::Acquire) {
        return Err(AppError::input("Application is closing."));
    }
    if state
        .capture
        .state
        .lock()
        .map_err(|_| AppError::input("Capture unavailable."))?
        .active
        || state.meeting_active.swap(true, Ordering::AcqRel)
    {
        return Err(AppError::input(
            "Stop the current recording before starting a meeting.",
        ));
    }
    crate::app::stop_playback(&app).await;
    state.cues.begin_capture();
    let id = id();
    let directory = state.meetings.root.join("audio").join(&id);
    let result = blocking(move || super::capture::Session::start(id, directory, options)).await;
    match result {
        Ok(session) => {
            *state
                .meeting_capture
                .lock()
                .map_err(|_| AppError::input("Capture unavailable."))? =
                Some(std::sync::Arc::new(session));
            changed(&app);
            Ok(())
        }
        Err(error) => {
            state.cues.idle();
            state.meeting_active.store(false, Ordering::Release);
            Err(error)
        }
    }
}
#[tauri::command]
pub fn meeting_mute(app: tauri::AppHandle, microphone: bool, muted: bool) -> Result<()> {
    let state = app.state::<AppState>();
    let capture = state
        .meeting_capture
        .lock()
        .map_err(|_| AppError::input("Capture unavailable."))?;
    let session = capture
        .as_ref()
        .ok_or_else(|| AppError::input("No meeting is recording."))?;
    if microphone {
        session.microphone_muted.store(muted, Ordering::Release);
    } else {
        session.system_muted.store(muted, Ordering::Release);
    }
    changed(&app);
    Ok(())
}
#[tauri::command]
pub async fn meeting_stop(app: tauri::AppHandle, discard: bool) -> Result<Option<String>> {
    let state = app.state::<AppState>();
    let _controls = state.controls.lock().await;
    finish_capture(&app, discard).await
}
pub async fn finish_capture(app: &tauri::AppHandle, discard: bool) -> Result<Option<String>> {
    let state = app.state::<AppState>();
    let session = state
        .meeting_capture
        .lock()
        .map_err(|_| AppError::input("Capture unavailable."))?
        .clone()
        .ok_or_else(|| AppError::input("No meeting is recording."))?;
    let id = session
        .status()
        .id
        .ok_or_else(|| AppError::input("Missing recording identity."))?;
    let directory = state.meetings.root.join("audio").join(&id);
    // Record discard intent before stopping; restart recovery must never submit it.
    if discard {
        let bytes = tokio::fs::read(directory.join("capture.json")).await?;
        let mut journal: super::capture::Journal = serde_json::from_slice(&bytes)?;
        journal.discarded = true;
        let path = directory.join("discard");
        blocking(move || crate::storage::publish_journal(&path, b"discard")).await?;
    }
    let finished = blocking(move || session.finish()).await;
    let failure = finished.as_ref().err().cloned();
    let result: Result<Option<String>> = async {
        if discard {
            tokio::fs::remove_dir_all(&directory).await?;
            return Ok(None);
        }
        let journal = match finished {
            Ok(j) => j,
            Err(_) => {
                serde_json::from_slice(&tokio::fs::read(directory.join("capture.json")).await?)?
            }
        };
        let assets = media::finalize_capture(&state.media, &directory, &journal).await?;
        let mut m = Meeting::new("Meeting recording".into(), SourceKind::Recording, None);
        m.id = id.clone();
        m.added_at = journal.created.clone();
        m.call_at = journal.created;
        m.duration = Some(journal.elapsed);
        m.capture_warning = journal.warning;

        m.audio = Some(Audio {
            download: "prepared.m4a".into(),
            prepared: Some("prepared.m4a".into()),
            assets,
        });
        if let Some(error) = failure {
            m.attempt.stage = Stage::Recovered;
            m.attempt.error = Some(error.message);
        }
        let store = state.meetings.clone();
        blocking(move || store.insert(m)).await?;
        let store = state.meetings.clone();
        let saved_id = id.clone();
        let _ = blocking(move || store.cleanup_capture_working_files(&saved_id)).await;
        state.meetings_wake.notify_one();
        Ok(Some(id))
    }
    .await;
    // Retain journal on finalization failure so it can be recovered.
    *state
        .meeting_capture
        .lock()
        .map_err(|_| AppError::input("Capture unavailable."))? = None;
    state.meeting_active.store(false, Ordering::Release);
    state.cues.idle();
    changed(app);
    result.map_err(|error| {
        if discard { error } else {
            AppError::new("meeting_save", &error.category,
                format!("{} Capture files are preserved. After resolving the problem, restart xTalk to recover this meeting.", error.message))
        }
    })
}

pub fn start_worker(
    app: tauri::AppHandle,
    store: MeetingStore,
    provider: std::sync::Arc<dyn super::providers::MeetingProvider>,
) {
    tauri::async_runtime::spawn(async move {
        let worker = super::jobs::Worker { store, provider };
        {
            let state = app.state::<AppState>();
            let _recovery_activity = state.worker_activity.read().await;
            // Capture recovery is local-only; the owner explicitly starts processing.
            let root = worker.store.root.join("audio");
            if let Ok(entries) = std::fs::read_dir(&root) {
                for entry in entries.flatten() {
                    let directory = entry.path();
                    if let Ok(bytes) = std::fs::read(directory.join("admission.json")) {
                        if let Ok(m) = serde_json::from_slice::<Meeting>(&bytes) {
                            if directory.file_name().and_then(|n| n.to_str()) == Some(&m.id) {
                                let id = m.id.clone();
                                if worker.store.recoverable(&id).unwrap_or(false) {
                                    let _ = worker.store.insert(m);
                                    changed(&app);
                                }
                                let _ = worker.store.cleanup_capture_working_files(&id);
                            }
                        }
                        continue;
                    }
                    if let Ok(bytes) = std::fs::read(directory.join("import.pending")) {
                        if let Ok(mut m) = serde_json::from_slice::<Meeting>(&bytes) {
                            if directory.file_name().and_then(|n| n.to_str()) == Some(&m.id)
                                && worker.store.recoverable(&m.id).unwrap_or(false)
                            {
                                m.attempt.stage = Stage::Failed;
                                m.attempt.error = Some("Import was interrupted before audio was saved. Import the original file again; no transcription was submitted.".into());
                                if worker.store.insert(m).is_ok() {
                                    let _ = std::fs::remove_dir_all(&directory);
                                    changed(&app);
                                }
                            }
                        }
                        continue;
                    }
                    let Ok(bytes) = std::fs::read(directory.join("capture.json")) else {
                        continue;
                    };
                    let Ok(journal) = serde_json::from_slice::<super::capture::Journal>(&bytes)
                    else {
                        continue;
                    };
                    if crate::domain::valid_id(&journal.id).is_err()
                        || directory.file_name().and_then(|n| n.to_str()) != Some(&journal.id)
                        || !worker.store.recoverable(&journal.id).unwrap_or(false)
                    {
                        continue;
                    }
                    if directory.join("discard").exists() {
                        let _ = std::fs::remove_dir_all(&directory);
                        continue;
                    }
                    if !journal.ready
                        || !journal.elapsed.is_finite()
                        || !(0.0..=MAX_SECONDS).contains(&journal.elapsed)
                        || journal.tracks.is_empty()
                        || journal.tracks.len() > 2
                        || journal.tracks.iter().any(|t| {
                            !["microphone.f32", "system.f32"].contains(&t.filename.as_str())
                                || !(8000..=96000).contains(&t.rate)
                                || !(1..=2).contains(&t.channels)
                        })
                    {
                        continue;
                    }
                    let state = app.state::<AppState>();
                    let Ok(assets) =
                        media::finalize_capture(&state.media, &directory, &journal).await
                    else {
                        continue;
                    };
                    let mut meeting =
                        Meeting::new("Recovered meeting".into(), SourceKind::Recording, None);
                    meeting.id = journal.id;
                    meeting.added_at = journal.created.clone();
                    meeting.call_at = journal.created;
                    meeting.duration = Some(journal.elapsed);
                    meeting.capture_warning = journal.warning;

                    meeting.audio = Some(Audio {
                        download: "prepared.m4a".into(),
                        prepared: Some("prepared.m4a".into()),
                        assets,
                    });
                    meeting.attempt.stage = Stage::Recovered;
                    meeting.attempt.error = Some("Recovered partial recording. Review its audio, then choose Re-transcribe to process it.".into());
                    let saved_id = meeting.id.clone();
                    if worker.store.insert(meeting).is_ok() {
                        let _ = worker.store.cleanup_capture_working_files(&saved_id);
                        changed(&app);
                    }
                }
            }
        }
        loop {
            let state = app.state::<AppState>();
            tokio::select! {
                _ = state.meetings_wake.notified() => {},
                _ = tokio::time::sleep(std::time::Duration::from_secs(10)) => {},
            }
            if state.shutting_down.load(Ordering::Acquire) {
                break;
            }
            let _activity = state.worker_activity.read().await;
            if let Ok(meetings) = worker.store.pending() {
                for m in meetings {
                    if state.shutting_down.load(Ordering::Acquire) {
                        break;
                    }
                    match worker.step(m).await {
                        Ok(true) => {
                            changed(&app);
                            state.meetings_wake.notify_one();
                        }
                        Ok(false) => {}
                        Err(error) => {
                            tracing::warn!(category = error.category, "meeting_worker_step_failed")
                        }
                    }
                }
            }
        }
    });
}
