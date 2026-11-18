//! Local, bounded diagnostics. Only application-owned metadata events enter this writer.
use crate::domain::*;
use std::{
    fs::{File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use tracing_appender::non_blocking::{ErrorCounter, NonBlocking, NonBlockingBuilder, WorkerGuard};
use tracing_subscriber::{fmt::writer::MakeWriterExt, prelude::*};
const MAX_FILE_BYTES: usize = 2 * 1024 * 1024;
const MAX_FILES: usize = 7;
#[derive(Clone)]
pub struct Diagnostics {
    directory: PathBuf,
    session: String,
    enabled: Arc<AtomicBool>,
    writer: NonBlocking,
    guard: Arc<Mutex<Option<WorkerGuard>>>,
    error: Arc<Mutex<Option<String>>>,
    dropped: ErrorCounter,
}
#[derive(serde::Serialize)]
pub struct Status {
    pub version: String,
    pub build_id: String,
    pub commit: String,
    pub directory: String,
    pub session: String,
    pub pid: u32,
    pub enabled: bool,
    pub error: Option<String>,
    pub dropped_events: usize,
}
impl Diagnostics {
    pub fn new(directory: PathBuf, enabled: bool) -> Self {
        let session = id();
        let error = Arc::new(Mutex::new(None));
        let files = Files {
            directory: directory.clone(),
            session: session.clone(),
            file: None,
            written: 0,
            sequence: 0,
            day: String::new(),
            error: error.clone(),
            limit: MAX_FILE_BYTES,
            keep: MAX_FILES,
        };
        let (writer, guard) = NonBlockingBuilder::default()
            .buffered_lines_limit(256)
            .lossy(true)
            .finish(files);
        let dropped = writer.error_counter();
        Self {
            directory,
            session,
            enabled: Arc::new(AtomicBool::new(enabled)),
            writer,
            guard: Arc::new(Mutex::new(Some(guard))),
            error,
            dropped,
        }
    }
    pub fn subscriber(&self) -> impl tracing::Subscriber + Send + Sync + 'static {
        let enabled = self.enabled.clone();
        // Writer filtering is evaluated for every event, including previously cached callsites.
        let writer = self
            .writer
            .clone()
            .with_filter(move |_| enabled.load(Ordering::Acquire));
        tracing_subscriber::registry().with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_thread_ids(true)
                .with_thread_names(true)
                .with_writer(writer)
                .with_filter(tracing_subscriber::filter::filter_fn(|metadata| {
                    metadata.target().starts_with("x_talk")
                        && *metadata.level() <= tracing::Level::INFO
                })),
        )
    }
    pub fn install(&self) {
        if self.subscriber().try_init().is_err() {
            *self.error.lock().unwrap() =
                Some("Diagnostic subscriber could not be initialized.".into());
            return;
        }
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            // Panic payloads may contain user content. Record source location only.
            if let Some(location) = info.location() {
                let file = Path::new(location.file())
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown");
                tracing::error!(file, line = location.line(), "application_panic");
            } else {
                tracing::error!("application_panic");
            }
            previous(info);
        }));
    }
    pub fn set_enabled(&self, enabled: bool) {
        if !enabled {
            tracing::info!(enabled, "diagnostic_logging_changed");
        }
        self.enabled.store(enabled, Ordering::Release);
        if enabled {
            tracing::info!(enabled, "diagnostic_logging_changed");
        }
    }
    pub fn status(&self) -> Status {
        Status {
            version: env!("CARGO_PKG_VERSION").into(),
            build_id: env!("XTALK_BUILD_ID").into(),
            commit: env!("XTALK_COMMIT").into(),
            directory: self.directory.to_string_lossy().into(),
            session: self.session.clone(),
            pid: std::process::id(),
            enabled: self.enabled.load(Ordering::Acquire),
            error: self.error.lock().ok().and_then(|e| e.clone()),
            dropped_events: self.dropped.dropped_lines(),
        }
    }
    pub fn directory(&self) -> &Path {
        &self.directory
    }
    /// Call off the event-loop thread. The appender's guard uses bounded shutdown waits.
    pub fn shutdown(&self) {
        let guard = self.guard.lock().ok().and_then(|mut guard| guard.take());
        drop(guard);
    }
}
/// Read just the logging preference before recovery, so even startup failures are diagnosable.
/// Old databases without this setting default on; a saved opt-out applies before any log event.
pub fn saved_preference(root: &Path) -> bool {
    let read = || -> Option<bool> {
        let db = rusqlite::Connection::open_with_flags(
            root.join("x-talk.sqlite3"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .ok()?;
        let document: String = db
            .query_row("SELECT document FROM settings WHERE id=1", [], |r| r.get(0))
            .ok()?;
        serde_json::from_str::<serde_json::Value>(&document)
            .ok()?
            .get("diagnostic_logging")?
            .as_bool()
    };
    read().unwrap_or(true)
}
struct Files {
    directory: PathBuf,
    session: String,
    file: Option<File>,
    written: usize,
    sequence: u32,
    day: String,
    error: Arc<Mutex<Option<String>>>,
    limit: usize,
    keep: usize,
}
impl Files {
    fn open(&mut self, day: String) -> io::Result<()> {
        self.file.take();
        std::fs::create_dir_all(&self.directory)?;
        self.sequence += 1;
        let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%S%.3fZ");
        let name = format!(
            "x-talk-{stamp}-{}-{}-{:04}.log",
            std::process::id(),
            &self.session[..8],
            self.sequence
        );
        let current = self.directory.join(&name);
        self.file = Some(
            OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&current)?,
        );
        self.written = 0;
        self.day = day;
        let mut files: Vec<_> = std::fs::read_dir(&self.directory)?
            .filter_map(|entry| {
                let entry = entry.ok()?;
                let name = entry.file_name();
                let name = name.to_str()?;
                if !name.starts_with("x-talk-")
                    || !name.ends_with(".log")
                    || !entry.file_type().ok()?.is_file()
                {
                    return None;
                }
                Some((entry.metadata().ok()?.modified().ok()?, entry.path()))
            })
            .collect();
        files.sort_by(|a, b| {
            (b.1 == current)
                .cmp(&(a.1 == current))
                .then_with(|| b.0.cmp(&a.0))
                .then_with(|| b.1.cmp(&a.1))
        });
        for (_, path) in files.into_iter().skip(self.keep) {
            let _ = std::fs::remove_file(path);
        }
        Ok(())
    }
}
impl Write for Files {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let result: io::Result<usize> = (|| {
            for chunk in bytes.chunks(self.limit) {
                let day = chrono::Utc::now().format("%Y-%m-%d").to_string();
                if self.file.is_none() || self.written + chunk.len() > self.limit || self.day != day
                {
                    self.open(day)?;
                }
                self.file.as_mut().unwrap().write_all(chunk)?;
                self.written += chunk.len();
            }
            Ok(bytes.len())
        })();
        if let Err(ref e) = result {
            self.file.take();
            if let Ok(mut error) = self.error.lock() {
                *error = Some(format!(
                    "Could not write diagnostic logs ({:?}, OS code {}).",
                    e.kind(),
                    e.raw_os_error().unwrap_or(0)
                ));
            }
        }
        result
    }
    fn flush(&mut self) -> io::Result<()> {
        if let Some(file) = &mut self.file {
            file.flush()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn contents(directory: &Path) -> String {
        if !directory.exists() {
            return String::new();
        }
        std::fs::read_dir(directory)
            .unwrap()
            .filter_map(|e| std::fs::read_to_string(e.unwrap().path()).ok())
            .collect()
    }
    fn event(stage: &str) {
        tracing::info!(stage, "diagnostic_probe");
    }
    #[test]
    fn toggles_apply_to_cached_events_and_external_payloads_are_excluded() {
        let dir = tempfile::tempdir().unwrap();
        let d = Diagnostics::new(dir.path().join("logs"), true);
        tracing::subscriber::with_default(d.subscriber(), || {
            event("before_toggle");
            d.set_enabled(false);
            event("disabled_sentinel");
            d.set_enabled(true);
            event("after_toggle");
            tracing::info!(target: "reqwest::headers", key = "private_key_sentinel", body = "private_body_sentinel", "external");
        });
        d.shutdown();
        let log = contents(d.directory());
        assert!(log.contains("before_toggle") && log.contains("after_toggle"));
        assert!(!log.contains("disabled_sentinel"));
        assert!(!log.contains("private_key_sentinel") && !log.contains("private_body_sentinel"));
        assert_eq!(d.status().dropped_events, 0);
    }
    #[test]
    fn disabled_startup_creates_no_log_files() {
        let dir = tempfile::tempdir().unwrap();
        let d = Diagnostics::new(dir.path().join("logs"), false);
        tracing::subscriber::with_default(d.subscriber(), || event("disabled_boot"));
        d.shutdown();
        assert!(!d.directory().exists());
    }
    #[test]
    fn rotation_bounds_file_size_and_count_and_keeps_unrelated_files() {
        let dir = tempfile::tempdir().unwrap();
        let unrelated = dir.path().join("keep.txt");
        std::fs::write(&unrelated, b"untouched").unwrap();
        let mut writer = Files {
            directory: dir.path().into(),
            session: id(),
            file: None,
            written: 0,
            sequence: 0,
            day: String::new(),
            error: Arc::default(),
            limit: 80,
            keep: 3,
        };
        for _ in 0..5 {
            writer.write_all(&[b'x'; 512]).unwrap();
        }
        let files: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|s| s == "log"))
            .collect();
        assert_eq!(files.len(), 3);
        assert!(files.iter().all(|p| p.metadata().unwrap().len() <= 80));
        assert_eq!(std::fs::read(unrelated).unwrap(), b"untouched");
    }
    #[test]
    fn logging_io_failure_is_visible_without_breaking_application_work() {
        let dir = tempfile::tempdir().unwrap();
        let blocked = dir.path().join("blocked");
        std::fs::write(&blocked, b"file in place of directory").unwrap();
        let d = Diagnostics::new(blocked, true);
        tracing::subscriber::with_default(d.subscriber(), || event("cannot_write"));
        d.shutdown();
        assert!(d
            .status()
            .error
            .as_deref()
            .unwrap()
            .contains("Could not write"));
    }
    #[test]
    fn legacy_preferences_default_on_and_saved_opt_out_is_read_before_startup() {
        let dir = tempfile::tempdir().unwrap();
        assert!(saved_preference(dir.path()));
        let store = crate::storage::Store::open(dir.path()).unwrap();
        store
            .call(|db| {
                let document: String =
                    db.query_row("SELECT document FROM settings WHERE id=1", [], |r| r.get(0))?;
                let mut old: serde_json::Value = serde_json::from_str(&document)?;
                old.as_object_mut().unwrap().remove("diagnostic_logging");
                db.execute(
                    "UPDATE settings SET document=?1 WHERE id=1",
                    [serde_json::to_string(&old)?],
                )?;
                Ok(())
            })
            .unwrap();
        assert!(store.settings().unwrap().diagnostic_logging);
        assert!(saved_preference(dir.path()));
        let mut settings = store.settings().unwrap();
        settings.diagnostic_logging = false;
        store.save_settings(settings).unwrap();
        assert!(!saved_preference(dir.path()));
        drop(store);
        let store = crate::storage::Store::open(dir.path()).unwrap();
        assert!(!store.settings().unwrap().diagnostic_logging);
    }
}
