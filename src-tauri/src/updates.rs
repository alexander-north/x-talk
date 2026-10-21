//! Native update ownership: the webview never supplies URLs, keys, or installer bytes.
use crate::{
    app::AppState,
    domain::{AppError, Result},
};
use serde::Serialize;
use std::sync::{atomic::Ordering, Mutex};
use std::time::Duration;
use tauri::{Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

#[derive(Clone, Default, Serialize)]
pub struct Status {
    pub phase: String,
    pub version: Option<String>,
    pub notes: Option<String>,
    pub error: Option<String>,
    pub downloaded: u64,
    pub total: Option<u64>,
}
#[derive(Default)]
pub struct Updates {
    status: Mutex<Status>,
    available: Mutex<Option<Update>>,
    operation: tokio::sync::Mutex<()>,
}
fn failure(message: &str) -> AppError {
    AppError::new("update", "unavailable", message)
}
fn change(app: &tauri::AppHandle, f: impl FnOnce(&mut Status)) {
    let service = app.state::<Updates>();
    let mut status = service.status.lock().unwrap();
    f(&mut status);
    let _ = app.emit("update-changed", status.clone());
}
#[tauri::command]
pub fn update_status(app: tauri::AppHandle) -> Status {
    app.state::<Updates>().status.lock().unwrap().clone()
}
#[tauri::command]
pub async fn check_update(app: tauri::AppHandle) -> Result<Status> {
    let service = app.state::<Updates>();
    let _operation = service
        .operation
        .try_lock()
        .map_err(|_| failure("An update operation is already in progress."))?;
    change(&app, |s| {
        s.phase = "checking".into();
        s.error = None;
    });
    let result = async {
        let builder = app.updater_builder().on_before_exit(|| {}).timeout(Duration::from_secs(20));
        #[cfg(all(debug_assertions, feature = "test-providers"))]
        let builder = if std::env::var("XTALK_TEST_PROVIDERS").as_deref() == Ok("1") {
            if let Ok(endpoint) = std::env::var("XTALK_TEST_UPDATE_URL") {
                let url: reqwest::Url = endpoint.parse().map_err(|_| failure("Invalid fixture URL"))?;
                if url.scheme() != "http" || url.host_str() != Some("127.0.0.1") {
                    return Err(failure("Update fixtures require HTTP loopback."));
                }
                builder.endpoints(vec![url]).map_err(|_| failure("Invalid fixture endpoint"))?
            } else { builder }
        } else { builder };
        let updater = builder.build()
            .map_err(|_| failure("Update checking is not configured correctly."))?;
        updater.check().await.map_err(|_| failure("Could not check for updates. Check your connection and try again. Updates are unavailable while the GitHub repository is private."))
    }.await;
    match result {
        Ok(update) => {
            change(&app, |s| {
                s.phase = if update.is_some() {
                    "available"
                } else {
                    "current"
                }
                .into();
                s.version = update.as_ref().map(|u| u.version.clone());
                s.notes = update.as_ref().and_then(|u| u.body.clone());
            });
            *service.available.lock().unwrap() = update;
            Ok(update_status(app.clone()))
        }
        Err(error) => {
            tracing::warn!("update_check_failed");
            change(&app, |s| {
                s.phase = "error".into();
                s.error = Some(error.message.clone());
            });
            Err(error)
        }
    }
}
#[tauri::command]
pub async fn install_update(app: tauri::AppHandle, confirmed: bool) -> Result<()> {
    if !confirmed {
        return Err(failure("Confirm Update and restart first."));
    }
    let updates = app.state::<Updates>();
    let _operation = updates
        .operation
        .try_lock()
        .map_err(|_| failure("An update operation is already in progress."))?;
    let update = updates
        .available
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| failure("Check for an update first."))?;
    let state = app.state::<AppState>();
    let _controls = state
        .controls
        .try_lock()
        .map_err(|_| failure("Wait for recording controls to finish."))?;
    let _worker = state
        .worker_activity
        .try_write()
        .map_err(|_| failure("Wait for processing or the transformation test to finish."))?;
    {
        let _gate = state
            .gate
            .lock()
            .map_err(|_| failure("Application is busy."))?;
        let active = state
            .capture
            .state
            .lock()
            .map_err(|_| failure("Recording state is unavailable."))?
            .active;
        if active
            || state.meeting_active.load(Ordering::Acquire)
            || state.meetings.update_busy()?
            || state.store.update_busy()?
            || state.test.lock().unwrap().is_some()
        {
            return Err(failure(
                "Finish recording, processing, imports, and audio playback/export before updating.",
            ));
        }
        if state.shutting_down.swap(true, Ordering::AcqRel) {
            return Err(failure("The application is already closing."));
        }
    }
    change(&app, |s| {
        s.phase = "downloading".into();
        s.error = None;
        s.downloaded = 0;
        s.total = None;
    });
    let result = async {
        // Verification is performed by the updater before any installer is launched.
        let bytes = tokio::time::timeout(Duration::from_secs(180), update.download(
            |chunk, total| change(&app, |s| { s.downloaded += chunk as u64; s.total = total; }), || {}
        )).await.map_err(|_| failure("The update download timed out. You can try again."))?
            .map_err(|_| failure("The update could not be downloaded or its signature could not be verified. Nothing was installed."))?;
        change(&app, |s| s.phase = "installing".into());
        let handle = app.clone();
        tauri::async_runtime::spawn_blocking(move || {
            use tauri_plugin_window_state::AppHandleExt;
            let _ = handle.save_window_state(tauri_plugin_window_state::StateFlags::all() & !tauri_plugin_window_state::StateFlags::VISIBLE);
            // NSIS restarts the app after handoff. Avoid destructive cleanup before
            // ShellExecute succeeds: a launch failure must leave this process usable.
            update.install(bytes).map_err(|_| failure("Could not start the update installer. Please try again or download it from GitHub Releases."))
        }).await.map_err(|_| failure("The update installer could not be started."))?
    }.await;
    // Windows exits on successful handoff. Errors must leave the current app usable.
    state.shutting_down.store(false, Ordering::Release);
    if let Err(ref error) = result {
        tracing::warn!("update_install_failed");
        change(&app, |s| {
            s.phase = "available".into();
            s.error = Some(error.message.clone());
        });
    }
    result
}

pub fn start(app: &tauri::AppHandle) {
    app.manage(Updates::default());
    if cfg!(debug_assertions) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(30)).await;
        loop {
            let state = app.state::<AppState>();
            if !state.shutting_down.load(Ordering::Acquire)
                && state.store.settings().is_ok_and(|s| s.check_updates)
            {
                let _ = check_update(app.clone()).await;
            }
            tokio::time::sleep(Duration::from_secs(12 * 60 * 60)).await;
        }
    });
}
