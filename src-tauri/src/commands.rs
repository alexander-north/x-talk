use crate::{
    app::{changed, AppState},
    capture::{Action, Capture, CaptureState, Device},
    credentials::PROVIDERS,
    domain::*,
    media::MediaProcessor,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tauri::Manager;
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_dialog::DialogExt;
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|_| {
        AppError::new(
            "application",
            "worker",
            "Application operation was interrupted.",
        )
    })?
}
fn operation_busy(s: &AppState) -> Result<bool> {
    Ok(s.test_active.load(Ordering::Acquire)
        || s.importing.load(Ordering::Acquire)
        || s.store.processing_busy()?)
}
fn operation_stage(s: &AppState) -> Option<String> {
    if s.test_active.load(Ordering::Acquire) {
        Some("transforming".into())
    } else if s.importing.load(Ordering::Acquire) {
        Some("importing".into())
    } else {
        None
    }
}
fn ensure_idle(s: &AppState) -> Result<()> {
    if operation_busy(s)?
        || s.capture
            .state
            .lock()
            .map_err(|_| AppError::input("Capture state unavailable."))?
            .active
    {
        return Err(AppError::input(
            "Finish or cancel the current operation first.",
        ));
    }
    Ok(())
}
#[derive(serde::Serialize)]
pub struct State {
    settings: Settings,
    transformation: Transformation,
    jobs: Vec<JobView>,
    capture: CaptureState,
    operation_busy: bool,
    operation_stage: Option<String>,
    devices: Vec<Device>,
    credentials: std::collections::BTreeMap<String, bool>,
    models: Vec<Model>,
    shortcut_error: Option<AppError>,
    recovery_warnings: Vec<AppError>,
    diagnostics: crate::diagnostics::Status,
    #[cfg(feature = "test-providers")]
    test_providers: bool,
    playing: Option<String>,
}
/// Frequent capture/job updates exclude devices, credentials and configuration payloads.
#[derive(serde::Serialize)]
pub struct Progress {
    jobs: Vec<JobView>,
    capture: CaptureState,
    operation_busy: bool,
    operation_stage: Option<String>,
    latest: Option<Entry>,
    playback: Option<PlaybackView>,
    diagnostics: crate::diagnostics::Status,
}
#[tauri::command]
pub async fn progress(app: tauri::AppHandle) -> Result<Progress> {
    #[cfg(feature = "test-providers")]
    tracing::info!("acceptance_progress_read");
    blocking(move || {
        let s = app.state::<AppState>();
        let capture = s
            .capture
            .state
            .lock()
            .map_err(|_| AppError::input("Capture state unavailable."))?
            .clone();
        Ok(Progress {
            jobs: s.store.jobs()?.into_iter().map(JobView::from).collect(),
            capture,
            operation_busy: operation_busy(&s)?,
            operation_stage: operation_stage(&s),
            latest: s.store.latest_entry()?,
            playback: playback_state(app.clone())?,
            diagnostics: s.diagnostics.status(),
        })
    })
    .await
}
#[tauri::command]
pub async fn state(app: tauri::AppHandle) -> Result<State> {
    #[cfg(feature = "test-providers")]
    tracing::info!("acceptance_metadata_read");
    blocking(move || {
        let s = app.state::<AppState>();
        let credentials = PROVIDERS
            .into_iter()
            .map(|p| (p.into(), s.keys.get(p).is_ok()))
            .collect();
        let result = Ok(State {
            settings: s.store.settings()?,
            recovery_warnings: s.store.recovery_warnings(),
            diagnostics: s.diagnostics.status(),
            #[cfg(feature = "test-providers")]
            test_providers: crate::test_support::enabled(),
            transformation: s.store.transformation()?,
            jobs: s.store.jobs()?.into_iter().map(JobView::from).collect(),
            capture: s
                .capture
                .state
                .lock()
                .map_err(|_| AppError::input("Capture state unavailable."))?
                .clone(),
            devices: Capture::devices().unwrap_or_default(),
            credentials,
            models: catalog(),
            shortcut_error: s
                .shortcut_error
                .lock()
                .map_err(|_| AppError::input("Shortcut state unavailable."))?
                .clone(),
            operation_busy: operation_busy(&s)?,
            operation_stage: operation_stage(&s),
            playing: s
                .playback
                .lock()
                .ok()
                .and_then(|p| p.as_ref().map(|p| p.id.clone())),
        });
        result
    })
    .await
}
#[tauri::command]
pub async fn capture_action(app: tauri::AppHandle, action: String) -> Result<()> {
    do_capture(&app, &action).await
}
pub async fn do_capture(app: &tauri::AppHandle, action: &str) -> Result<()> {
    if !["start", "toggle", "stop", "cancel"].contains(&action) {
        return Err(AppError::input("Invalid capture action."));
    }
    tracing::info!(action, "capture_command_received");
    let service = app.state::<AppState>();
    if service.shutting_down.load(Ordering::Acquire) && ["start", "toggle"].contains(&action) {
        return Err(AppError::input(
            "Application is closing or installing an update.",
        ));
    }
    let _controls = service.controls.lock().await;
    tracing::info!(action, "capture_controls_acquired");
    if ["start", "toggle"].contains(&action) {
        crate::app::stop_playback(app).await;
    }
    let app = app.clone();
    let action = action.to_owned();
    blocking(move || {
        let s = app.state::<AppState>();
        let _gate = s
            .gate
            .lock()
            .map_err(|_| AppError::input("Application state is unavailable."))?;
        if s.shutting_down.load(Ordering::Acquire) && ["start", "toggle"].contains(&action.as_str())
        {
            return Err(AppError::input("Application is shutting down."));
        }
        let active = s
            .capture
            .state
            .lock()
            .map_err(|_| AppError::input("Capture state is unavailable."))?
            .active;
        let command = if action == "cancel" {
            if !active {
                s.test_cancel.store(true, Ordering::Release);
                s.import_cancel.store(true, Ordering::Release);
                s.store.cancel_processing()?;
                changed(&app);
                return Ok(());
            }
            Action::Cancel
        } else if action == "stop" || (action == "toggle" && active) {
            Action::Stop
        } else {
            ensure_idle(&s)?;
            let settings = s.store.settings()?;
            let snapshot = s.store.snapshot()?;
            s.keys.get(&snapshot.transcription.provider)?;
            if let Some(p) = &snapshot.preset {
                s.keys.get(&p.provider)?;
            }
            Action::Start {
                snapshot,
                device: settings.microphone,
            }
        };
        let starting = matches!(&command, Action::Start { .. }) && !active;
        if starting {
            s.cues.begin_capture();
        }
        let result = s.capture.action(command);
        if starting && result.is_err() {
            s.cues.idle();
        }
        result
    })
    .await
}
#[tauri::command]
pub async fn history(app: tauri::AppHandle, offset: u32) -> Result<Vec<Entry>> {
    blocking(move || app.state::<AppState>().store.entries(offset)).await
}
#[tauri::command]
pub async fn entry(app: tauri::AppHandle, id: String) -> Result<Entry> {
    blocking(move || app.state::<AppState>().store.entry(&id)).await
}
#[tauri::command]
pub async fn import_audio(app: tauri::AppHandle) -> Result<()> {
    tracing::info!("import_requested");
    let reservation = id();
    let app2 = app.clone();
    let reserve = reservation.clone();
    let (snapshot, source) = blocking(move || {
        let s = app2.state::<AppState>();
        let snapshot;
        {
            let _gate = s
                .gate
                .lock()
                .map_err(|_| AppError::input("Application busy."))?;
            if s.shutting_down.load(Ordering::Acquire) {
                return Err(AppError::input(
                    "Application is closing or installing an update.",
                ));
            }
            ensure_idle(&s)?;
            snapshot = s.store.snapshot()?;
            s.keys.get(&snapshot.transcription.provider)?;
            if let Some(p) = &snapshot.preset {
                s.keys.get(&p.provider)?;
            }
            s.store.reserve(&reserve)?;
            s.import_cancel.store(false, Ordering::Release);
            s.importing.store(true, Ordering::Release);
            changed(&app2);
        }
        let selected = app2
            .dialog()
            .file()
            .add_filter("Audio", &["wav", "mp3", "m4a", "aac", "flac"])
            .blocking_pick_file();
        let path = selected.and_then(|p| p.into_path().ok());
        Ok((snapshot, path))
    })
    .await?;
    let s = app.state::<AppState>();
    let result: Result<()> = tokio::select! { result = async {
        let Some(source) = source else { return Ok(()) };
        use tokio::io::AsyncReadExt;
        let _ = s.media.probe(&source).await?;
        let source_ext = source
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("audio")
            .to_ascii_lowercase();
        let ext = if ["wav", "mp3", "m4a", "aac", "flac"].contains(&source_ext.as_str()) {
            source_ext.as_str()
        } else {
            "audio"
        };
        let relative = format!("audio/{reservation}.{ext}");
        let dest = s.store.path(&relative)?;
        let part = s
            .store
            .root
            .join("audio")
            .join(format!("{reservation}.import.part"));
        let copy_result = async {
            let mut input = tokio::fs::File::open(&source).await?.take(1_073_741_825);
            let mut output = tokio::fs::File::create(&part).await?;
            let bytes = tokio::io::copy(&mut input, &mut output).await?;
            output.sync_all().await?;
            drop(output);
            if bytes > 1_073_741_824 {
                return Err(AppError::input(
                    "Audio source exceeds the 1 GiB import limit.",
                ));
            }
            let info = s.media.probe(&part).await?;
            let check_dir = s.store.root.join("tmp").join(&reservation);
            let mut validation = snapshot.clone();
            validation.compression = false;
            validation.silence_removal = false;
            let check = s.media.prepare(&part, &check_dir, &validation).await;
            let _ = tokio::fs::remove_dir_all(check_dir).await;
            if let Err(e) = check {
                if e.category != "no_speech" {
                    return Err(e);
                }
            }
            let title = source
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("Imported audio")
                .to_owned();
            let e = Entry::new(
                reservation.clone(),
                "import",
                title,
                Some(relative),
                info.duration,
                bytes,
            );
            let j = Job::new(e.id.clone(), "import", snapshot, false);
            // Publish and admit as one gated, non-yielding step. Cancellation must
            // never drop an asynchronous rename between publication and admission.
            let _gate = s.gate.lock().map_err(|_| AppError::input("Application busy."))?;
            if s.import_cancel.load(Ordering::Acquire) {
                return Err(AppError::input("Import canceled. Your source file is preserved."));
            }
            s.store.stage_admission(&e, &j)?;
            std::fs::rename(&part, &dest)?;
            s.store.admit(e, j, Some(reservation.clone()))?;
            s.store.finish_admission(&reservation)?;
            Ok(())
        }
        .await;
        if copy_result.is_err() && s.store.entry(&reservation).is_err() {
            let _ = tokio::fs::remove_file(&part).await;
            let _ = tokio::fs::remove_file(&dest).await;
            let _ = s.store.finish_admission(&reservation);
        }
        copy_result?;
        tracing::info!(entry_id = reservation, "import_admitted");
        s.wake.notify_one();
        changed(&app);
        Ok(())
    }
    => result,
        _ = async { while !s.import_cancel.load(Ordering::Acquire) { tokio::time::sleep(std::time::Duration::from_millis(30)).await; } } => Err(AppError::input("Import canceled. Your source file is preserved."))
    };
    let _ = tokio::fs::remove_dir_all(s.store.root.join("tmp").join(&reservation)).await;
    if s.store.entry(&reservation).is_err() {
        let _ = tokio::fs::remove_file(
            s.store
                .root
                .join("audio")
                .join(format!("{reservation}.import.part")),
        )
        .await;
        let _ = s.store.finish_admission(&reservation);
    }
    let _ = s.store.release(&reservation);
    s.importing.store(false, Ordering::Release);
    changed(&app);
    if let Err(e) = &result {
        tracing::warn!(stage = e.stage, category = e.category, "import_failed");
    }
    result
}
#[tauri::command]
pub async fn retry(app: tauri::AppHandle, id: String, revision: u64, kind: String) -> Result<()> {
    blocking(move || {
        let s = app.state::<AppState>();
        let _gate = s
            .gate
            .lock()
            .map_err(|_| AppError::input("Application busy."))?;
        if s.shutting_down.load(Ordering::Acquire) {
            return Err(AppError::input(
                "Application is closing or installing an update.",
            ));
        }
        ensure_idle(&s)?;
        let mut settings = s.store.settings()?;
        if kind == "transform" {
            settings.transform = true;
        }
        let snap = Snapshot::capture(&settings, s.store.preset(&settings.preset_id)?);
        if kind == "transcribe" {
            s.keys.get(&snap.transcription.provider)?;
        } else if let Some(p) = &snap.preset {
            s.keys.get(&p.provider)?;
        }
        s.store.retry(&id, revision, &kind, snap)?;
        s.wake.notify_one();
        changed(&app);
        Ok(())
    })
    .await
}
#[tauri::command]
pub async fn edit_metadata(
    app: tauri::AppHandle,
    id: String,
    revision: u64,
    title: String,
    subtitle: String,
) -> Result<()> {
    blocking(move || {
        app.state::<AppState>()
            .store
            .edit(&id, revision, title, subtitle)?;
        changed(&app);
        Ok(())
    })
    .await
}
#[tauri::command]
pub async fn recordings(app: tauri::AppHandle, query: RecordingQuery) -> Result<RecordingPage> {
    blocking(move || app.state::<AppState>().store.recordings(query)).await
}
#[tauri::command]
pub async fn edit_recording(
    app: tauri::AppHandle,
    id: String,
    revision: u64,
    title: String,
    subtitle: String,
    transcript: String,
) -> Result<()> {
    blocking(move || {
        app.state::<AppState>()
            .store
            .edit_recording(&id, revision, title, subtitle, transcript)?;
        tracing::info!("recording_edited");
        changed(&app);
        Ok(())
    })
    .await
}
#[tauri::command]
pub async fn delete_entry(
    app: tauri::AppHandle,
    id: String,
    revision: u64,
    confirmed: bool,
) -> Result<()> {
    blocking(move || {
        app.state::<AppState>()
            .store
            .delete(&id, revision, confirmed)?;
        changed(&app);
        Ok(())
    })
    .await
}
#[tauri::command]
pub async fn clear_recordings(app: tauri::AppHandle, confirmed: bool) -> Result<u64> {
    if !confirmed {
        return Err(AppError::input("Confirm clearing all recordings first."));
    }
    let service = app.state::<AppState>();
    let _controls = service.controls.lock().await;
    crate::app::stop_playback(&app).await;
    let handle = app.clone();
    blocking(move || {
        let s = handle.state::<AppState>();
        let _gate = s
            .gate
            .lock()
            .map_err(|_| AppError::input("Application busy."))?;
        if s.shutting_down.load(Ordering::Acquire) {
            return Err(AppError::input(
                "Application is closing or installing an update.",
            ));
        }
        if s.capture
            .state
            .lock()
            .map_err(|_| AppError::input("Capture state unavailable."))?
            .active
        {
            return Err(AppError::input(
                "Stop or cancel recording before clearing the library.",
            ));
        }
        let result = s.store.clear_recordings(confirmed);
        if let Ok(count) = &result {
            tracing::info!(count = *count, "recordings_cleared");
        }
        // Refresh even if managed-file cleanup needs a later retry after logical deletion.
        changed(&handle);
        result
    })
    .await
}
#[tauri::command]
pub async fn export_audio(app: tauri::AppHandle, id: String) -> Result<()> {
    blocking(move || {
        let s = app.state::<AppState>();
        let e = {
            let _gate = s
                .gate
                .lock()
                .map_err(|_| AppError::input("Application busy."))?;
            if s.shutting_down.load(Ordering::Acquire) {
                return Err(AppError::input(
                    "Application is closing or installing an update.",
                ));
            }
            s.store.pin_audio(&id)?
        };
        let result = (|| {
            let relative = e
                .audio
                .ok_or_else(|| AppError::input("Original audio is no longer retained."))?;
            let source = s.store.path(&relative)?;
            let name = format!(
                "x-talk-{}.{}",
                &id[..8],
                source.extension().and_then(|e| e.to_str()).unwrap_or("wav")
            );
            if let Some(dest) = app
                .dialog()
                .file()
                .set_file_name(name)
                .blocking_save_file()
                .and_then(|p| p.into_path().ok())
            {
                if dest == source {
                    return Err(AppError::input(
                        "Choose a location outside app-managed storage.",
                    ));
                }
                std::fs::copy(source, dest)?;
            }
            Ok(())
        })();
        s.store.lease(&id, false)?;
        s.store.retention()?;
        result
    })
    .await
}
#[tauri::command]
pub async fn copy_text(
    app: tauri::AppHandle,
    id: Option<String>,
    kind: Option<String>,
    text: Option<String>,
) -> Result<()> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    let text = if let Some(id) = id {
        let e = app.state::<AppState>().store.entry(&id)?;
        match kind.as_deref() {
            Some("transcript") => e.transcript,
            Some("transformed") => e.transformed,
            _ => None,
        }
        .ok_or_else(|| AppError::input("Requested text is unavailable."))?
    } else {
        text.filter(|t| t.len() <= 2_000_000)
            .ok_or_else(|| AppError::input("No test output to copy."))?
    };
    tracing::info!("clipboard_copy_started");
    for _ in 0..4 {
        if app.clipboard().write_text(&text).is_ok() {
            tracing::info!("clipboard_copy_completed");
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(75)).await;
    }
    tracing::warn!("clipboard_copy_failed");
    Err(AppError::new(
        "delivery",
        "clipboard_busy",
        "Clipboard is busy. Try Copy again.",
    ))
}
#[tauri::command]
pub async fn save_settings(app: tauri::AppHandle, settings: Settings) -> Result<Settings> {
    blocking(move || {
        let s = app.state::<AppState>();
        let _gate = s
            .gate
            .lock()
            .map_err(|_| AppError::input("Application busy."))?;
        if s.shutting_down.load(Ordering::Acquire) {
            return Err(AppError::input(
                "Application is closing or installing an update.",
            ));
        }
        settings.validate()?;
        let old = s.store.settings()?;
        if old.revision != settings.revision {
            return Err(AppError::input("Settings changed. Reload before saving."));
        }
        if settings.preset_id != old.preset_id {
            return Err(AppError::input(
                "The app uses one transformation configuration.",
            ));
        }
        crate::app::shortcuts(&app, Some(&old.shortcuts), &settings.shortcuts)?;
        let effects = (|| {
            if settings.autostart != old.autostart {
                if settings.autostart {
                    app.autolaunch().enable()
                } else {
                    app.autolaunch().disable()
                }
                .map_err(|_| {
                    AppError::new(
                        "settings",
                        "autostart",
                        "Windows login startup could not be changed.",
                    )
                })?;
            }
            if let Some(w) = app.get_webview_window("main") {
                w.set_always_on_top(settings.topmost)
                    .map_err(|_| AppError::input("Always-on-top could not be changed."))?;
            }
            s.store.save_settings(settings.clone())
        })();
        if effects.is_err() {
            let _ = crate::app::shortcuts(&app, Some(&settings.shortcuts), &old.shortcuts);
            if old.autostart {
                let _ = app.autolaunch().enable();
            } else {
                let _ = app.autolaunch().disable();
            }
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.set_always_on_top(old.topmost);
            }
        } else {
            s.diagnostics.set_enabled(settings.diagnostic_logging);
            tracing::info!(revision = settings.revision, "settings_saved");
            *s.shortcut_error.lock().unwrap() = None;
            let _ = s.store.retention();
            crate::app::configuration_changed(&app);
        }
        effects
    })
    .await
}
#[tauri::command]
pub async fn save_transformation(
    app: tauri::AppHandle,
    transformation: Transformation,
) -> Result<Transformation> {
    blocking(move || {
        let s = app.state::<AppState>();
        let _gate = s
            .gate
            .lock()
            .map_err(|_| AppError::input("Application busy."))?;
        if s.shutting_down.load(Ordering::Acquire) {
            return Err(AppError::input(
                "Application is closing or installing an update.",
            ));
        }
        let result = s.store.save_transformation(transformation)?;
        crate::app::configuration_changed(&app);
        Ok(result)
    })
    .await
}
#[tauri::command]
pub async fn credential(
    app: tauri::AppHandle,
    provider: String,
    action: String,
    key: Option<String>,
) -> Result<()> {
    blocking(move || {
        let s = app.state::<AppState>();
        match action.as_str() {
            "set" => s.keys.set(
                &provider,
                key.as_deref()
                    .ok_or_else(|| AppError::input("Enter a key."))?,
            )?,
            "delete" => s.keys.delete(&provider)?,
            _ => return Err(AppError::input("Invalid credential action.")),
        }
        tracing::info!(provider, action, "credential_changed");
        crate::app::configuration_changed(&app);
        Ok(())
    })
    .await
}
#[tauri::command]
pub async fn test_transformation(
    app: tauri::AppHandle,
    transformation: Transformation,
    input: String,
) -> Result<String> {
    let preset = transformation.with_metadata(Preset::seed())?;
    if input.trim().is_empty() || input.len() > 1_000_000 {
        return Err(AppError::input("Enter test text of at most 1 MB."));
    }
    let rx = {
        let s = app.state::<AppState>();
        let _gate = s
            .gate
            .lock()
            .map_err(|_| AppError::input("Application busy."))?;
        if s.shutting_down.load(Ordering::Acquire) {
            return Err(AppError::input(
                "Application is closing or installing an update.",
            ));
        }
        ensure_idle(&s)?;
        s.keys.get(&preset.provider)?;
        let mut test = s
            .test
            .lock()
            .map_err(|_| AppError::input("Test panel unavailable."))?;
        if test.is_some() {
            return Err(AppError::input("One preset test is already waiting."));
        }
        let (tx, rx) = tokio::sync::oneshot::channel();
        *test = Some(crate::app::PresetTest {
            preset,
            input,
            cutoff: s.store.last_sequence()?,
            reply: tx,
        });
        s.test_cancel.store(false, Ordering::Release);
        s.test_active.store(true, Ordering::Release);
        changed(&app);
        s.wake.notify_one();
        rx
    };

    rx.await
        .map_err(|_| AppError::input("Preset test was interrupted."))?
}
#[tauri::command]
pub async fn recover_jobs(app: tauri::AppHandle, ids: Vec<String>, discard: bool) -> Result<()> {
    blocking(move || {
        let s = app.state::<AppState>();
        let _gate = s
            .gate
            .lock()
            .map_err(|_| AppError::input("Application busy."))?;
        if s.shutting_down.load(Ordering::Acquire) {
            return Err(AppError::input(
                "Application is closing or installing an update.",
            ));
        }
        if !discard {
            ensure_idle(&s)?;
        }
        s.store.resume(ids, discard)?;
        s.store.retention()?;
        s.wake.notify_one();
        changed(&app);
        Ok(())
    })
    .await
}
#[tauri::command]
pub async fn stop_playback(app: tauri::AppHandle) -> Result<()> {
    let service = app.state::<AppState>();
    let _controls = service.controls.lock().await;
    crate::app::stop_playback(&app).await;
    changed(&app);
    Ok(())
}
#[tauri::command]
pub async fn play_audio(
    app: tauri::AppHandle,
    id: String,
    start_at: Option<f64>,
    paused: Option<bool>,
) -> Result<()> {
    let start = start_at.unwrap_or(0.);
    if !start.is_finite() || !(0. ..=MAX_SECONDS).contains(&start) {
        return Err(AppError::input("Invalid playback position."));
    }
    let service = app.state::<AppState>();
    if service.shutting_down.load(Ordering::Acquire) {
        return Err(AppError::input(
            "Application is closing or installing an update.",
        ));
    }
    let _controls = service.controls.lock().await;
    if service.shutting_down.load(Ordering::Acquire) {
        return Err(AppError::input("Application is shutting down."));
    }
    crate::app::stop_playback(&app).await;
    let s = app.state::<AppState>();
    if s.capture
        .state
        .lock()
        .map_err(|_| AppError::input("Capture state unavailable."))?
        .active
    {
        return Err(AppError::input("Stop recording before playing audio."));
    }
    let e = s.store.pin_audio(&id)?;
    let path = s.store.path(
        &e.audio
            .ok_or_else(|| AppError::input("Original audio is no longer retained."))?,
    )?;
    let stop = Arc::new(AtomicBool::new(false));
    let done = Arc::new(tokio::sync::Notify::new());
    let control = Arc::new(crate::capture::PlaybackControl::new(
        start.min(e.duration),
        paused.unwrap_or(false),
    ));
    *s.playback
        .lock()
        .map_err(|_| AppError::input("Playback state unavailable."))? =
        Some(crate::app::Playback {
            id: id.clone(),
            stop: stop.clone(),
            done: done.clone(),
            control: control.clone(),
            duration: e.duration,
        });
    let media = s.media.clone();
    let store = s.store.clone();
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        tracing::info!(entry_id = id, "playback_started");
        let result =
            crate::capture::play(&media, &path, stop.clone(), control, start.min(e.duration)).await;
        if let Err(e) = result {
            tracing::warn!(entry_id = id, category = e.category, "playback_failed");
            let _ = tauri::Emitter::emit(&handle, "playback-error", e);
        }
        tracing::info!(entry_id = id, "playback_finished");
        let _ = store.lease(&id, false);
        let _ = store.retention();
        if let Ok(mut current) = handle.state::<AppState>().playback.lock() {
            if current
                .as_ref()
                .is_some_and(|p| Arc::ptr_eq(&p.stop, &stop))
            {
                *current = None;
            }
        }
        done.notify_one();
        changed(&handle);
    });
    changed(&app);
    Ok(())
}
#[derive(serde::Serialize)]
pub struct PlaybackView {
    id: String,
    paused: bool,
    position: f64,
    duration: f64,
}
#[tauri::command]
pub fn playback_state(app: tauri::AppHandle) -> Result<Option<PlaybackView>> {
    let s = app.state::<AppState>();
    let playback = s
        .playback
        .lock()
        .map_err(|_| AppError::input("Playback state unavailable."))?;
    Ok(playback.as_ref().map(|p| PlaybackView {
        id: p.id.clone(),
        paused: p.control.paused.load(Ordering::Acquire),
        position: p.control.position().min(p.duration),
        duration: p.duration,
    }))
}
#[tauri::command]
pub fn pause_playback(app: tauri::AppHandle, id: String, paused: bool) -> Result<()> {
    let s = app.state::<AppState>();
    let playback = s
        .playback
        .lock()
        .map_err(|_| AppError::input("Playback state unavailable."))?;
    let current = playback
        .as_ref()
        .filter(|p| p.id == id)
        .ok_or_else(|| AppError::input("Audio is no longer playing."))?;
    current.control.paused.store(paused, Ordering::Release);
    tracing::info!(paused, "playback_pause_changed");
    changed(&app);
    Ok(())
}
#[tauri::command]
pub async fn window_mode(app: tauri::AppHandle, mode: String) -> Result<()> {
    blocking(move || {
        let window = app
            .get_webview_window("main")
            .ok_or_else(|| AppError::input("Window unavailable."))?;
        let change = || -> tauri::Result<()> {
            if mode == "fullscreen" {
                return window.set_fullscreen(!window.is_fullscreen()?);
            }
            window.set_fullscreen(false)?;
            window.unmaximize()?;
            window.set_min_size(Some(tauri::LogicalSize::new(380., 360.)))?;
            let (width, height): (f64, f64) = if mode == "compact" {
                (420., 380.)
            } else {
                (1100., 780.)
            };
            let (width, height) = if let Some(monitor) = window.current_monitor()? {
                let bounds = monitor.size().to_logical::<f64>(monitor.scale_factor());
                (
                    width.min(bounds.width),
                    height.min((bounds.height - 64.).max(420.)),
                )
            } else {
                (width, height)
            };
            window.set_size(tauri::LogicalSize::new(width, height))?;
            window.center()
        };
        if !["compact", "full", "fullscreen"].contains(&mode.as_str()) {
            return Err(AppError::input("Unsupported window mode."));
        }
        change().map_err(|_| AppError::input("Could not change the window size."))?;
        tracing::info!(mode, "window_mode_changed");
        Ok(())
    })
    .await
}
#[tauri::command]
pub async fn open_repository() -> Result<()> {
    blocking(|| {
        const URL: &str = "https://github.com/alexander-north/x-talk";
        #[cfg(windows)]
        std::process::Command::new("rundll32.exe")
            .args(["url.dll,FileProtocolHandler", URL])
            .spawn()?;
        #[cfg(not(windows))]
        std::process::Command::new("xdg-open").arg(URL).spawn()?;
        Ok(())
    })
    .await
}
#[tauri::command]
pub fn preview_sound(app: tauri::AppHandle, event: String) -> Result<()> {
    if ![
        "started",
        "stopped",
        "canceled",
        "transcribed",
        "transformed",
    ]
    .contains(&event.as_str())
    {
        return Err(AppError::input("Unknown event sound."));
    }
    crate::app::play_sound(&app, &event, true)
}
#[tauri::command]
pub async fn open_logs_folder(app: tauri::AppHandle) -> Result<()> {
    blocking(move || {
        let path = app
            .state::<AppState>()
            .diagnostics
            .directory()
            .to_path_buf();
        std::fs::create_dir_all(&path)?;
        #[cfg(windows)]
        {
            std::process::Command::new("explorer.exe")
                .arg(path)
                .spawn()?;
        }
        #[cfg(not(windows))]
        {
            std::process::Command::new("xdg-open").arg(path).spawn()?;
        }
        tracing::info!("logs_folder_opened");
        Ok(())
    })
    .await
}
#[tauri::command]
pub async fn quit(app: tauri::AppHandle, discard_capture: bool) -> Result<()> {
    let s = app.state::<AppState>();
    if s.shutting_down.swap(true, Ordering::AcqRel) {
        return Ok(());
    }
    let started = std::time::Instant::now();
    tracing::info!(discard = discard_capture, "shutdown_requested");
    tracing::info!("shutdown_playback_stop_started");
    crate::app::stop_playback(&app).await;
    tracing::info!("shutdown_playback_stop_completed");
    tracing::info!("shutdown_capture_stop_started");
    let stopped = tokio::time::timeout(
        std::time::Duration::from_secs(8),
        do_capture(&app, if discard_capture { "cancel" } else { "stop" }),
    )
    .await
    .unwrap_or_else(|_| {
        Err(AppError::new(
            "shutdown",
            "timeout",
            "Recording cleanup did not finish. Recovery files are preserved; try Quit again.",
        ))
    });
    if let Err(e) = stopped {
        tracing::error!(stage = e.stage, category = e.category, "shutdown_failed");
        s.shutting_down.store(false, Ordering::Release);
        return Err(e);
    }
    tracing::info!("shutdown_capture_stop_completed");
    s.cues.shutdown();
    let handle = app.clone();
    let save = tauri::async_runtime::spawn_blocking(move || {
        use tauri_plugin_window_state::AppHandleExt;
        handle.save_window_state(
            tauri_plugin_window_state::StateFlags::all()
                & !tauri_plugin_window_state::StateFlags::VISIBLE,
        )
    });
    tracing::info!("shutdown_window_state_started");
    if !matches!(
        tokio::time::timeout(std::time::Duration::from_secs(1), save).await,
        Ok(Ok(Ok(())))
    ) {
        tracing::warn!("shutdown_window_state_incomplete");
    }
    tracing::info!(
        duration_ms = started.elapsed().as_millis() as u64,
        "shutdown_ready_to_exit"
    );
    let diagnostics = s.diagnostics.clone();
    let flush = tauri::async_runtime::spawn_blocking(move || diagnostics.shutdown());
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), flush).await;
    s.exit_ready.store(true, Ordering::Release);
    app.exit(0);
    Ok(())
}

#[tauri::command]
pub fn shortcut_editing(app: tauri::AppHandle, editing: bool) -> Result<()> {
    let focused = app
        .get_webview_window("main")
        .and_then(|w| w.is_focused().ok())
        .unwrap_or(false);
    app.state::<AppState>()
        .shortcut_editing
        .store(editing && focused, std::sync::atomic::Ordering::Release);
    Ok(())
}
