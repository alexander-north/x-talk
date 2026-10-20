use crate::{
    capture::{Capture, CaptureFeedback},
    credentials::{CredentialStore, NativeCredentials},
    diagnostics::Diagnostics,
    domain::*,
    jobs::{ClipboardWriter, Feedback, Worker},
    media::Ffmpeg,
    providers::{Providers, TranscriptionProvider, TransformationProvider},
    storage::Store,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use tauri::{Emitter, Manager};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
use tokio::sync::{oneshot, Notify};
pub struct PresetTest {
    pub input: String,
    pub preset: Preset,
    pub cutoff: i64,
    pub reply: oneshot::Sender<Result<String>>,
}
pub struct AppState {
    pub meeting_capture: Mutex<Option<Arc<crate::meetings::capture::Session>>>,
    pub meeting_active: AtomicBool,
    pub meetings: crate::meetings::store::MeetingStore,
    pub meetings_wake: Arc<Notify>,
    pub meeting_import: Mutex<Option<crate::meetings::commands::PendingImport>>,
    pub meeting_import_gate: tokio::sync::Mutex<()>,
    pub store: Store,
    pub cues: crate::cues::CuePlayer,
    pub capture: Capture,
    pub media: Arc<Ffmpeg>,
    pub keys: Arc<dyn CredentialStore>,
    pub wake: Arc<Notify>,
    pub test: Arc<Mutex<Option<PresetTest>>>,
    pub gate: Mutex<()>,
    pub controls: tokio::sync::Mutex<()>,
    pub worker_activity: tokio::sync::RwLock<()>,
    pub playback: Mutex<Option<Playback>>,
    pub shutting_down: AtomicBool,
    pub exit_ready: AtomicBool,
    pub test_active: AtomicBool,
    pub test_cancel: AtomicBool,
    pub importing: AtomicBool,
    pub import_cancel: AtomicBool,
    pub diagnostics: Diagnostics,
    pub shortcut_error: Mutex<Option<AppError>>,
    pub shortcut_editing: AtomicBool,
}
pub struct Playback {
    pub id: String,
    pub stop: Arc<AtomicBool>,
    pub done: Arc<Notify>,
    pub control: Arc<crate::capture::PlaybackControl>,
    pub duration: f64,
}
struct DesktopFeedback {
    app: tauri::AppHandle,
    wake: Arc<Notify>,
}
impl Feedback for DesktopFeedback {
    fn changed(&self) {
        changed(&self.app)
    }
    fn sound(&self, e: &str) {
        if let Some(s) = self.app.try_state::<AppState>() {
            self.sound_in_context(e, s.cues.context());
        }
    }
    fn sound_context(&self, job: &Job) -> Option<u64> {
        self.app
            .try_state::<AppState>()
            .and_then(|s| s.cues.job_context(job.seq))
    }
    fn sound_in_context(&self, e: &str, context: Option<u64>) {
        if let Some(s) = self.app.try_state::<AppState>() {
            if !s.shutting_down.load(Ordering::Acquire) && !s.meeting_active.load(Ordering::Acquire)
            {
                s.cues.submit(e, false, context);
            }
        }
    }
}
impl CaptureFeedback for DesktopFeedback {
    fn finalizing(&self) {
        if let Some(s) = self.app.try_state::<AppState>() {
            if let Ok(mut state) = s.capture.state.lock() {
                state.finalizing = true;
            }
        }
        changed(&self.app);
    }
    fn spectrum(&self, bands: [f32; crate::spectrum::BANDS]) {
        let _ = self.app.emit("capture-spectrum", bands.as_slice());
    }
    fn changed(&self) {
        changed(&self.app)
    }
    fn event(&self, e: &str) {
        if let Some(s) = self.app.try_state::<AppState>() {
            if e == "idle" {
                s.cues.idle();
            } else if !s.shutting_down.load(Ordering::Acquire)
                && !s.meeting_active.load(Ordering::Acquire)
            {
                s.cues.submit(e, false, None);
            }
        }
        let active = e == "started";
        if let Some(tray) = self.app.tray_by_id("main") {
            let _ = tray.set_icon(Some(tray_image(active)));
            let _ = tray.set_tooltip(Some(if active {
                "x-talk · Recording"
            } else {
                "x-talk · Ready"
            }));
        }
    }
    fn queued(&self) {
        self.wake.notify_one();
    }
}
struct DesktopClipboard(tauri::AppHandle);
#[async_trait::async_trait]
impl ClipboardWriter for DesktopClipboard {
    async fn copy(&self, text: &str) -> Result<()> {
        for _ in 0..4 {
            if self.0.clipboard().write_text(text).is_ok() {
                return Ok(());
            }
            tokio::time::sleep(std::time::Duration::from_millis(75)).await;
        }
        Err(AppError::new(
            "delivery",
            "clipboard_busy",
            "Clipboard is busy. Copy the saved result from history.",
        ))
    }
}
pub fn configuration_changed(app: &tauri::AppHandle) {
    let _ = app.emit("configuration-changed", ());
}
pub fn changed(app: &tauri::AppHandle) {
    let _ = app.emit("state-changed", ());
}
pub(crate) fn tray_image(active: bool) -> tauri::image::Image<'static> {
    if !active {
        return tauri::image::Image::from_bytes(include_bytes!("../icons/icon.png"))
            .expect("bundled app icon");
    }
    let mut data = vec![0u8; 32 * 32 * 4];
    for y in 0..32 {
        for x in 0..32 {
            let idx = (y * 32 + x) * 4;
            let inside = (x as i32 - 16).pow(2) + (y as i32 - 16).pow(2) < 170;
            if inside {
                data[idx..idx + 4].copy_from_slice(if active {
                    &[240, 75, 87, 255]
                } else {
                    &[75, 197, 153, 255]
                });
            }
        }
    }
    tauri::image::Image::new_owned(data, 32, 32)
}
pub fn play_sound(app: &tauri::AppHandle, event: &str, preview: bool) -> Result<()> {
    let s = app.state::<AppState>();
    if s.meeting_active.load(Ordering::Acquire)
        || s.shutting_down.load(Ordering::Acquire)
        || !s.cues.submit(event, preview, s.cues.context())
    {
        return Err(AppError::input(
            "Sound previews are unavailable while recording or closing.",
        ));
    }
    Ok(())
}
pub fn show(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}
pub fn shortcuts(app: &tauri::AppHandle, old: Option<&Shortcuts>, new: &Shortcuts) -> Result<()> {
    let parse = |s: &str| {
        s.parse::<Shortcut>().map_err(|_| {
            AppError::input("Invalid shortcut. Use a combination such as Ctrl+Alt+Space.")
        })
    };
    let parse_all = |s: &Shortcuts| -> Result<Vec<Shortcut>> {
        [&s.toggle, &s.cancel]
            .into_iter()
            .filter(|v| !v.trim().is_empty())
            .map(|v| parse(v.trim()))
            .collect()
    };
    let new = parse_all(new)?;
    if new.len() == 2 && new[0] == new[1] {
        return Err(AppError::input("Shortcuts must be distinct."));
    }
    let old = old.map(parse_all).transpose()?.unwrap_or_default();
    let mut registered = Vec::new();
    for &shortcut in &new {
        if old.contains(&shortcut) {
            continue;
        }
        if app.global_shortcut().register(shortcut).is_err() {
            for s in registered {
                let _ = app.global_shortcut().unregister(s);
            }
            return Err(AppError::new(
                "shortcuts",
                "conflict",
                "A shortcut is already in use. Existing shortcuts were kept.",
            ));
        }
        registered.push(shortcut);
    }
    for s in old {
        if !new.contains(&s) {
            let _ = app.global_shortcut().unregister(s);
        }
    }
    Ok(())
}
pub fn trigger(app: &tauri::AppHandle, action: &str) {
    let app = app.clone();
    let action = action.to_owned();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = crate::commands::do_capture(&app, &action).await {
            let s = app.state::<AppState>();
            if let Ok(mut state) = s.capture.state.lock() {
                state.error = Some(e);
            }
            changed(&app);
        }
    });
}
pub async fn stop_playback(app: &tauri::AppHandle) {
    let s = app.state::<AppState>();
    let current = s.playback.lock().ok().and_then(|mut p| p.take());
    if let Some(p) = current {
        tracing::info!(entry_id = p.id, "playback_stop_requested");
        p.stop.store(true, Ordering::Release);
        let _ = tokio::time::timeout(std::time::Duration::from_secs(3), p.done.notified()).await;
    }
}
pub fn run() {
    let boot = std::time::Instant::now();
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            tracing::info!("second_instance_activated");
            show(app);
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--background"]),
        ))
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(
                    tauri_plugin_window_state::StateFlags::all()
                        & !tauri_plugin_window_state::StateFlags::VISIBLE,
                )
                .build(),
        )
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    use std::collections::HashSet;
                    use std::sync::OnceLock;
                    static DOWN: OnceLock<Mutex<HashSet<u32>>> = OnceLock::new();
                    let down = DOWN.get_or_init(|| Mutex::new(HashSet::new()));
                    let Ok(mut down) = down.lock() else { return };
                    if event.state == ShortcutState::Released {
                        down.remove(&shortcut.id());
                        return;
                    }
                    if !down.insert(shortcut.id()) {
                        return;
                    }
                    drop(down);
                    let Some(s) = app.try_state::<AppState>() else {
                        return;
                    };
                    let Ok(settings) = s.store.settings() else {
                        return;
                    };
                    for (name, key) in [
                        ("toggle", settings.shortcuts.toggle),
                        ("cancel", settings.shortcuts.cancel),
                    ] {
                        if key.trim().parse::<Shortcut>().is_ok_and(|s| s == *shortcut) {
                            if s.shortcut_editing.load(Ordering::Acquire) {
                                let _ = app.emit("shortcut-captured", key);
                            } else {
                                trigger(app, name);
                            }
                            break;
                        }
                    }
                })
                .build(),
        )
        .setup(move |app| {
            let root = app.path().app_local_data_dir()?;
            #[cfg(debug_assertions)]
            let root = std::env::var_os("XTALK_TEST_DATA_DIR")
                .map(std::path::PathBuf::from)
                .unwrap_or(root);
            #[cfg(feature = "test-providers")]
            if !crate::test_support::enabled() {
                return Err("Acceptance builds require explicit test providers and an isolated test data directory.".into());
            }
            let logs = std::env::temp_dir().join("x-talk").join("logs");
            #[cfg(debug_assertions)]
            let logs = if std::env::var_os("XTALK_TEST_DATA_DIR").is_some() { root.join("logs") } else { logs };
            let diagnostics = Diagnostics::new(logs, crate::diagnostics::saved_preference(&root));
            diagnostics.install();
            tracing::info!(version = env!("CARGO_PKG_VERSION"), pid = std::process::id(),
                session = diagnostics.status().session, build_id = env!("XTALK_BUILD_ID"),
                debug = cfg!(debug_assertions), test_build = cfg!(feature = "test-providers"), "application_start");
            tracing::info!("storage_open_started");
            let store = Store::open(&root).inspect_err(|e| {
                tracing::error!(stage = e.stage, category = e.category, "storage_open_failed");
            })?;
            tracing::info!("storage_open_completed");
            diagnostics.set_enabled(store.settings()?.diagnostic_logging);
            #[cfg(feature = "test-providers")]
            if crate::test_support::enabled() {
                let mut settings = store.settings()?;
                settings.autostart = false;
                settings.shortcuts.toggle = "Ctrl+Alt+Shift+F7".into();
                settings.shortcuts.cancel = "Ctrl+Alt+Shift+F9".into();
                store.save_settings(settings)?;
            }
            let wake = Arc::new(Notify::new());
            let feedback = Arc::new(DesktopFeedback {
                app: app.handle().clone(),
                wake: wake.clone(),
            });
            let keys: Arc<dyn CredentialStore> = Arc::new(NativeCredentials);
            #[cfg(feature = "test-providers")]
            let keys: Arc<dyn CredentialStore> = if crate::test_support::enabled() {
                Arc::new(crate::test_support::TestProviders)
            } else {
                keys
            };
            let providers = Arc::new(Providers::new(keys.clone())?);
            let streaming: Arc<dyn crate::realtime::StreamingProvider> = Arc::new(crate::realtime::ElevenLabs { keys: keys.clone() });
            #[cfg(feature = "test-providers")]
            let streaming: Arc<dyn crate::realtime::StreamingProvider> = if crate::test_support::enabled() {
                Arc::new(crate::test_support::TestProviders)
            } else { streaming };
            let capture = Capture::new(store.clone(), feedback.clone(), streaming)?;
            let transcription: Arc<dyn TranscriptionProvider> = providers.clone();
            let transformation: Arc<dyn TransformationProvider> = providers.clone();
            #[cfg(feature = "test-providers")]
            let (transcription, transformation): (
                Arc<dyn TranscriptionProvider>,
                Arc<dyn TransformationProvider>,
            ) = if crate::test_support::enabled() {
                (
                    Arc::new(crate::test_support::TestProviders),
                    Arc::new(crate::test_support::TestProviders),
                )
            } else {
                (transcription, transformation)
            };
            #[cfg(any(not(debug_assertions), windows))]
            let tools = app.path().resource_dir()?.join("binaries");
            #[cfg(any(not(debug_assertions), windows))]
            let media = Arc::new(Ffmpeg {
                ffmpeg: tools.join("ffmpeg.exe"),
                ffprobe: tools.join("ffprobe.exe"),
            });
            #[cfg(all(debug_assertions, not(windows)))]
            let media = Arc::new(Ffmpeg {
                ffmpeg: "/usr/bin/ffmpeg".into(),
                ffprobe: "/usr/bin/ffprobe".into(),
            });
            let processing_media: Arc<dyn crate::media::MediaProcessor> = media.clone();
            #[cfg(feature = "test-providers")]
            let processing_media: Arc<dyn crate::media::MediaProcessor> =
                if crate::test_support::enabled() {
                    Arc::new(crate::test_support::TestProviders)
                } else {
                    processing_media
                };
            let worker = Worker {
                store: store.clone(),
                media: processing_media,
                transcription,
                transformation: transformation.clone(),
                clipboard: Arc::new(DesktopClipboard(app.handle().clone())),
                feedback,
            };
            let tests = Arc::new(Mutex::new(None::<PresetTest>));
            let meetings = crate::meetings::store::MeetingStore::open(&root.join("meetings"))?;
            let meeting_provider: Arc<dyn crate::meetings::providers::MeetingProvider> = Arc::new(Providers::new(keys.clone())?);
            #[cfg(feature = "test-providers")]
            let meeting_provider: Arc<dyn crate::meetings::providers::MeetingProvider> = if crate::test_support::enabled() {
                Arc::new(crate::test_support::TestProviders)
            } else { meeting_provider };
            app.manage(AppState {
                meeting_capture: Mutex::new(None),
                meeting_active: AtomicBool::new(false),
                meetings: meetings.clone(),
                meetings_wake: Arc::new(Notify::new()),
                meeting_import: Mutex::new(None),
                meeting_import_gate: tokio::sync::Mutex::new(()),
                store: store.clone(),
                cues: crate::cues::CuePlayer::new(store.clone())?,
                capture,
                media,
                keys,
                wake: wake.clone(),
                test: tests.clone(),
                gate: Mutex::new(()),
                controls: tokio::sync::Mutex::new(()),
                worker_activity: tokio::sync::RwLock::new(()),
                playback: Mutex::new(None),
                shutting_down: AtomicBool::new(false),
                exit_ready: AtomicBool::new(false),
                test_active: AtomicBool::new(false),
                test_cancel: AtomicBool::new(false),
                importing: AtomicBool::new(false),
                import_cancel: AtomicBool::new(false),
                diagnostics,
                shortcut_error: Mutex::new(None),
                shortcut_editing: AtomicBool::new(false),
            });
            crate::meetings::commands::start_worker(app.handle().clone(), meetings, meeting_provider);
            crate::updates::start(app.handle());
            let settings = store.settings()?;
            if let Err(e) = shortcuts(app.handle(), None, &settings.shortcuts) {
                *app.state::<AppState>().shortcut_error.lock().unwrap() = Some(e);
            }
            if settings.autostart {
                if app.autolaunch().enable().is_err() {
                    tracing::warn!("autostart_enable_failed");
                }
            } else {
                let _ = app.autolaunch().disable();
            }
            if let Some(w) = app.get_webview_window("main") {
                w.set_always_on_top(settings.topmost)?;
            }
            use tauri::menu::{MenuBuilder, MenuItemBuilder};
            let mut menu = MenuBuilder::new(app);
            for (id, title) in [
                ("open", "Open x-talk"),
                ("toggle", "Start / stop recording"),
                ("stop", "Stop recording"),
                ("stop-meeting", "Stop and save meeting"),
                ("cancel", "Cancel current operation"),
                ("quit", "Quit x-talk"),
            ] {
                menu = menu.item(&MenuItemBuilder::with_id(id, title).build(app)?);
            }
            let menu = menu.build()?;
            tauri::tray::TrayIconBuilder::with_id("main")
                .icon(tray_image(false))
                .tooltip("x-talk · Ready")
                .menu(&menu)
                .on_menu_event(|app, e| match e.id().as_ref() {
                    "open" => show(app),
                    "stop-meeting" => {
                        let handle = app.clone();
                        tauri::async_runtime::spawn(async move {
                            if let Err(error) = crate::meetings::commands::meeting_stop(handle.clone(), false).await {
                                let _ = handle.emit("playback-error", error);
                            }
                        });
                    },
                    "toggle" | "stop" | "cancel" => trigger(app, e.id().as_ref()),
                    "quit" => {
                        show(app);
                        let _ = app.emit("quit-requested", ());
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, e| {
                    if matches!(e, tauri::tray::TrayIconEvent::DoubleClick { .. }) {
                        show(tray.app_handle());
                    }
                })
                .build(app)?;
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    wake.notified().await;
                    loop {
                        let service = handle.state::<AppState>();
                        let _activity = service.worker_activity.read().await;
                        if handle
                            .state::<AppState>()
                            .shutting_down
                            .load(Ordering::Acquire)
                        {
                            break;
                        }
                        let test = tests.lock().ok().and_then(|mut t| t.take());
                        if let Some(test) = test {
                            if let Err(e) = worker.drain_limit(Some(test.cutoff)).await {
                                service.test_active.store(false, Ordering::Release);
                                let _ = test.reply.send(Err(e));
                                changed(&handle);
                                break;
                            }
                            let result = tokio::select! {
                                result = transformation.transform(&test.input, &test.preset) => result,
                                _ = async { while !service.test_cancel.load(Ordering::Acquire) { tokio::time::sleep(std::time::Duration::from_millis(30)).await; } } => Err(AppError::input("Transformation test canceled."))
                            };
                            service.test_active.store(false, Ordering::Release);
                            let _ = test.reply.send(result);
                            changed(&handle);
                            continue;
                        }
                        match worker.run_next().await {
                            Ok(true) => {}
                            Ok(false) => break,
                            Err(e) => {
                                tracing::error!(category = e.category, "worker_stopped");
                                break;
                            }
                        }
                    }
                }
            });
            store.retention()?;
            tracing::info!(
                startup_ms = boot.elapsed().as_millis() as u64,
                "desktop_ready"
            );
            if !std::env::args().any(|a| a == "--background") {
                show(app.handle());
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if matches!(event, tauri::WindowEvent::Focused(false) | tauri::WindowEvent::CloseRequested { .. }) {
                window.state::<AppState>().shortcut_editing.store(false, Ordering::Release);
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let s = window.state::<AppState>();
                if s.shutting_down.load(Ordering::Acquire) {
                    if !s.exit_ready.load(Ordering::Acquire) { api.prevent_close(); }
                    return;
                }
                api.prevent_close();
                if s.store.settings().is_ok_and(|s| s.close_to_tray) {
                    let _ = window.hide();
                } else {
                    let _ = window.emit("quit-requested", ());
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            crate::meetings::commands::meeting_play,
            crate::meetings::commands::meeting_devices,
            crate::meetings::commands::meeting_capture_status,
            crate::meetings::commands::meeting_start,
            crate::meetings::commands::meeting_stop,
            crate::meetings::commands::meeting_mute,
            crate::meetings::commands::meetings_list,
            crate::meetings::commands::meeting_get,
            crate::meetings::commands::meeting_pick,
            crate::meetings::commands::meeting_import,
            crate::meetings::commands::meeting_edit,
            crate::meetings::commands::meeting_copy,
            crate::meetings::commands::meeting_download,
            crate::meetings::commands::meeting_delete,
            crate::meetings::commands::meeting_retention,
            crate::meetings::commands::meeting_set_retention,
            crate::commands::state,
            crate::commands::shortcut_editing,
            crate::commands::progress,
            crate::commands::capture_action,
            crate::commands::import_audio,
            crate::commands::history,
            crate::commands::entry,
            crate::commands::retry,
            crate::commands::edit_metadata,
            crate::commands::edit_recording,
            crate::commands::recordings,
            crate::commands::delete_entry,
            crate::commands::clear_recordings,
            crate::commands::export_audio,
            crate::commands::play_audio,
            crate::commands::stop_playback,
            crate::commands::pause_playback,
            crate::commands::playback_state,
            crate::commands::window_mode,
            crate::commands::open_repository,
            crate::commands::preview_sound,
            crate::commands::copy_text,
            crate::commands::save_settings,
            crate::commands::save_transformation,
            crate::commands::credential,
            crate::commands::test_transformation,
            crate::commands::recover_jobs,
            crate::commands::quit,
            crate::updates::update_status,
            crate::updates::check_update,
            crate::updates::install_update,
            crate::commands::open_logs_folder
        ]);
    let app = builder
        .build(tauri::generate_context!())
        .expect("x-talk could not initialize; inspect local app data and dependencies");
    app.run(|app, event| {
        if let tauri::RunEvent::ExitRequested { api, .. } = event {
            let s = app.state::<AppState>();
            if !s.exit_ready.load(Ordering::Acquire) {
                api.prevent_exit();
                if !s.shutting_down.load(Ordering::Acquire) {
                    let app = app.clone();
                    tauri::async_runtime::spawn(async move {
                        let _ = crate::commands::quit(app, false).await;
                    });
                }
            }
            // Never wait for capture here: its tray feedback requires this event loop.
        }
    });
}
