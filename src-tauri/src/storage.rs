use crate::domain::*;
use rusqlite::{params, Connection, OptionalExtension};
use std::{
    path::{Path, PathBuf},
    sync::{mpsc, Arc, Mutex},
};
type Work = Box<dyn FnOnce(&mut Connection) + Send>;
#[derive(Clone)]
pub struct Store {
    worker: Arc<DatabaseWorker>,
    pub root: Arc<PathBuf>,
    recovery_warnings: Arc<Mutex<Vec<AppError>>>,
}
struct DatabaseWorker {
    tx: Option<mpsc::Sender<Work>>,
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
}
impl Drop for DatabaseWorker {
    fn drop(&mut self) {
        self.tx.take();
        if let Ok(thread) = self.thread.get_mut() {
            if let Some(thread) = thread.take() {
                if thread.thread().id() != std::thread::current().id() {
                    let _ = thread.join();
                }
            }
        }
    }
}
impl Store {
    pub fn open(root: &Path) -> Result<Self> {
        std::fs::create_dir_all(root.join("audio"))?;
        std::fs::create_dir_all(root.join("tmp"))?;
        let path = root.join("x-talk.sqlite3");
        let mut db = Connection::open(&path)?;
        db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA busy_timeout=5000;")?;
        let version: u32 = db.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version > 2 {
            return Err(AppError::new(
                "storage",
                "version",
                "This database was created by a newer x-talk version.",
            ));
        }
        if version == 0 {
            if path.metadata()?.len() > 4096 {
                db.backup(
                    rusqlite::MAIN_DB,
                    root.join("before-migration.sqlite3"),
                    None,
                )?;
            }
            let tx = db.transaction()?;
            tx.execute_batch(include_str!("../migrations/001.sql"))?;
            tx.execute(
                "INSERT INTO settings VALUES(1,?1)",
                [serde_json::to_string(&Settings::default())?],
            )?;
            let p = Preset::seed();
            tx.execute(
                "INSERT INTO presets VALUES(?1,?2,?3)",
                params![p.id, p.name, serde_json::to_string(&p)?],
            )?;
            tx.commit()?;
        }
        if version < 2 {
            if version > 0 {
                db.backup(
                    rusqlite::MAIN_DB,
                    root.join("before-migration-v2.sqlite3"),
                    None,
                )?;
            }
            let tx = db.transaction()?;
            tx.execute_batch(include_str!("../migrations/002.sql"))?;
            tx.commit()?;
        }
        db.execute_batch("DELETE FROM reservations; DELETE FROM audio_leases;")?;
        let (tx, rx) = mpsc::channel::<Work>();
        let thread = std::thread::Builder::new()
            .name("database".into())
            .spawn(move || {
                for work in rx {
                    work(&mut db)
                }
            })?;
        let store = Self {
            worker: Arc::new(DatabaseWorker {
                tx: Some(tx),
                thread: Mutex::new(Some(thread)),
            }),
            root: Arc::new(root.into()),
            recovery_warnings: Arc::default(),
        };
        store.recover()?;
        store.recover_admissions()?;
        store.reconcile()?;
        Ok(store)
    }
    pub fn recovery_warnings(&self) -> Vec<AppError> {
        self.recovery_warnings
            .lock()
            .map(|v| v.clone())
            .unwrap_or_default()
    }
    pub(crate) fn warn_recovery(&self) {
        tracing::warn!(
            category = "incomplete_recovery",
            "recovery_artifact_preserved"
        );
        if let Ok(mut warnings) = self.recovery_warnings.lock() {
            if warnings.is_empty() {
                warnings.push(AppError::new("recovery", "incomplete_recovery",
                    "Some interrupted audio could not be recovered automatically. Its audio and recovery records are preserved in the app's audio folder for manual recovery. Existing history and new recordings remain available."));
            }
        }
    }
    pub(crate) fn quarantine(&self, path: &Path) {
        self.warn_recovery();
        let mut name = path.as_os_str().to_os_string();
        name.push(format!(".{}.invalid", id()));
        // If renaming fails, leave the original untouched and retry recovery next launch.
        let _ = std::fs::rename(path, PathBuf::from(name));
    }
    pub fn stage_admission(&self, entry: &Entry, job: &Job) -> Result<()> {
        valid_id(&entry.id)?;
        valid_id(&job.id)?;
        if job.entry_id != entry.id {
            return Err(AppError::input("Import recovery identifiers do not match."));
        }
        let path = self
            .root
            .join("audio")
            .join(format!("{}.admission", entry.id));
        publish_journal(&path, &serde_json::to_vec(&(entry, job))?)
    }
    pub fn finish_admission(&self, id: &str) -> Result<()> {
        valid_id(id)?;
        let path = self.root.join("audio").join(format!("{id}.admission"));
        for artifact in [&path, &path.with_extension("admission.new")] {
            if artifact.exists() {
                std::fs::remove_file(artifact)?;
            }
        }
        Ok(())
    }
    fn recover_admission(&self, path: &Path, identifier: &str) -> Result<()> {
        let (entry, mut job): (Entry, Job) = serde_json::from_slice(&std::fs::read(path)?)?;
        valid_id(&entry.id)?;
        valid_id(&job.id)?;
        if entry.id != identifier
            || job.entry_id != entry.id
            || !((entry.source == "import" && job.kind == "import")
                || (entry.source == "microphone"
                    && job.kind == "initial"
                    && job.snapshot.transcription_mode == TranscriptionMode::Realtime
                    && job.realtime.is_some()))
        {
            return Err(AppError::input("Import recovery identifiers do not match."));
        }
        if self.entry(&entry.id).is_ok() {
            std::fs::remove_file(path)?;
            return Ok(());
        }
        let audio = self.path(
            entry
                .audio
                .as_deref()
                .ok_or_else(|| AppError::input("Import audio is missing."))?,
        )?;
        if audio.parent() != Some(self.root.join("audio").as_path())
            || audio.file_stem().and_then(|name| name.to_str()) != Some(entry.id.as_str())
        {
            return Err(AppError::input(
                "Import recovery audio identifier does not match.",
            ));
        }
        let part = self
            .root
            .join("audio")
            .join(format!("{}.import.part", entry.id));
        // A valid journal is written only after the complete copy has been synced and validated.
        if !audio.is_file() && part.is_file() {
            std::fs::rename(&part, &audio)?;
        }
        if !audio.is_file() {
            return Err(AppError::input("Import audio is missing."));
        }
        job.status = "paused".into();
        job.eligible = false;
        job.delivery = "manual_only".into();
        self.admit(entry, job, None)?;
        std::fs::remove_file(path)?;
        Ok(())
    }
    fn recover_admissions(&self) -> Result<()> {
        for item in std::fs::read_dir(self.root.join("audio"))? {
            let path = item?.path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.ends_with(".invalid") {
                self.warn_recovery();
                continue;
            }
            let Some(identifier) = name
                .strip_suffix(".admission")
                .or_else(|| name.strip_suffix(".admission.new"))
            else {
                continue;
            };
            if valid_id(identifier).is_err() || self.recover_admission(&path, identifier).is_err() {
                self.quarantine(&path);
            }
        }
        // Without a valid published journal, a copy's completeness is unknown. Preserve it.
        for item in std::fs::read_dir(self.root.join("audio"))? {
            let path = item?.path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.ends_with(".import.part") {
                self.warn_recovery();
            }
        }
        Ok(())
    }
    pub fn call<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let (tx, rx) = mpsc::sync_channel(1);
        self.worker
            .tx
            .as_ref()
            .ok_or_else(|| AppError::new("storage", "closed", "Database worker stopped."))?
            .send(Box::new(move |db| {
                let _ = tx.send(f(db));
            }))
            .map_err(|_| AppError::new("storage", "closed", "Database worker stopped."))?;
        rx.recv()
            .map_err(|_| AppError::new("storage", "closed", "Database worker stopped."))?
    }
    pub fn next_sequence(&self) -> Result<Option<i64>> {
        self.call(|db| {
            Ok(
                db.query_row("SELECT min(seq) FROM jobs WHERE status='queued'", [], |r| {
                    r.get(0)
                })?,
            )
        })
    }
    pub fn last_sequence(&self) -> Result<i64> {
        self.call(|db| Ok(db.query_row("SELECT coalesce(max(seq),0) FROM jobs", [], |r| r.get(0))?))
    }
    pub fn settings(&self) -> Result<Settings> {
        self.call(|db| {
            let s: String =
                db.query_row("SELECT document FROM settings WHERE id=1", [], |r| r.get(0))?;
            Ok(serde_json::from_str(&s)?)
        })
    }
    pub fn save_settings(&self, mut s: Settings) -> Result<Settings> {
        s.validate()?;
        self.call(move |db| {
            let current: String =
                db.query_row("SELECT document FROM settings WHERE id=1", [], |r| r.get(0))?;
            let old: Settings = serde_json::from_str(&current)?;
            if old.revision != s.revision {
                return Err(AppError::input("Settings changed. Reload before saving."));
            }
            let exists: bool = db.query_row(
                "SELECT EXISTS(SELECT 1 FROM presets WHERE id=?1)",
                [&s.preset_id],
                |r| r.get(0),
            )?;
            if !exists {
                return Err(AppError::input("Select an existing preset."));
            }
            s.revision += 1;
            db.execute(
                "UPDATE settings SET document=?1 WHERE id=1",
                [serde_json::to_string(&s)?],
            )?;
            Ok(s)
        })
    }
    pub fn transformation(&self) -> Result<Transformation> {
        self.call(|db| Ok(Transformation::from(read_active_preset(db)?)))
    }
    pub fn save_transformation(&self, config: Transformation) -> Result<Transformation> {
        self.call(move |db| {
            let tx = db.transaction()?;
            let previous = read_active_preset(&tx)?;
            if previous.revision != config.revision {
                return Err(AppError::input(
                    "Transformation changed. Reopen settings before saving.",
                ));
            }
            let mut updated = config.with_metadata(previous)?;
            updated.revision += 1;
            tx.execute(
                "UPDATE presets SET document=?1 WHERE id=?2",
                params![serde_json::to_string(&updated)?, updated.id],
            )?;
            tx.commit()?;
            Ok(Transformation::from(updated))
        })
    }
    pub fn presets(&self) -> Result<Vec<Preset>> {
        self.call(|db| {
            let mut q = db.prepare("SELECT document FROM presets ORDER BY name COLLATE NOCASE")?;
            let rows = q.query_map([], |r| r.get::<_, String>(0))?;
            rows.map(|r| Ok(serde_json::from_str(&r?)?)).collect()
        })
    }
    pub fn preset(&self, id: &str) -> Result<Preset> {
        let id = id.to_owned();
        self.call(move |db| {
            let s: String =
                db.query_row("SELECT document FROM presets WHERE id=?1", [id], |r| {
                    r.get(0)
                })?;
            Ok(serde_json::from_str(&s)?)
        })
    }
    pub fn save_preset(&self, mut p: Preset) -> Result<Preset> {
        p.validate()?;
        self.call(move|db|{
let old:Option<String>=db.query_row("SELECT document FROM presets WHERE id=?1",[&p.id],|r|r.get(0)).optional()?;
if let Some(old)=old {
if serde_json::from_str::<Preset>(&old)?.revision!=p.revision{
return Err(AppError::input("Preset changed. Reload before saving."));
}
}
p.revision+=1;
db.execute("INSERT INTO presets VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET name=excluded.name,document=excluded.document",params![p.id,p.name,serde_json::to_string(&p)?]).map_err(|_|AppError::input("Preset names must be unique."))?;
Ok(p)}
)
    }
    pub fn delete_preset(&self, id: &str, revision: u64) -> Result<()> {
        let id = id.to_owned();
        self.call(move |db| {
            let current: String =
                db.query_row("SELECT document FROM presets WHERE id=?1", [&id], |r| {
                    r.get(0)
                })?;
            if serde_json::from_str::<Preset>(&current)?.revision != revision {
                return Err(AppError::input("Preset changed. Reload before deleting."));
            }

            let s: String =
                db.query_row("SELECT document FROM settings WHERE id=1", [], |r| r.get(0))?;
            if serde_json::from_str::<Settings>(&s)?.preset_id == id {
                return Err(AppError::input(
                    "Select another preset before deleting this one.",
                ));
            }
            db.execute("DELETE FROM presets WHERE id=?1", [id])?;
            Ok(())
        })
    }
    pub fn snapshot(&self) -> Result<Snapshot> {
        let s = self.settings()?;
        Ok(Snapshot::capture(&s, self.preset(&s.preset_id)?))
    }
    pub fn latest_entry(&self) -> Result<Option<Entry>> {
        self.call(|db| {
            let document: Option<String> = db
                .query_row(
                    "SELECT document FROM entries WHERE deleting=0 ORDER BY seq DESC LIMIT 1",
                    [],
                    |r| r.get(0),
                )
                .optional()?;
            document
                .map(|value| serde_json::from_str(&value).map_err(Into::into))
                .transpose()
        })
    }
    pub fn entries(&self, offset: u32) -> Result<Vec<Entry>> {
        self.call(move|db|{
let mut q=db.prepare("SELECT document FROM entries WHERE deleting=0 ORDER BY seq DESC LIMIT 30 OFFSET ?1")?;
let rows=q.query_map([offset],|r|r.get::<_,String>(0))?;
rows.map(|r|Ok(serde_json::from_str(&r?)?)).collect()}
)
    }
    pub fn entry(&self, id: &str) -> Result<Entry> {
        valid_id(id)?;
        let id = id.to_owned();
        self.call(move |db| read_entry(db, &id))
    }
    pub fn recordings(&self, query: RecordingQuery) -> Result<RecordingPage> {
        if ![25, 50, 100].contains(&query.page_size) {
            return Err(AppError::input(
                "Choose 25, 50, or 100 recordings per page.",
            ));
        }
        if query.search.len() > 2000 {
            return Err(AppError::input("Search must be at most 2000 bytes."));
        }
        let field = match query.sort.as_str() {
            "created" | "title" | "subtitle" | "transcript" | "transformed" | "source"
            | "duration" | "bytes" | "transcript_model" | "transform_model" | "id" => {
                query.sort.clone()
            }
            _ => return Err(AppError::input("Unsupported recordings sort column.")),
        };
        self.call(move |db| {
            let filter = "deleting=0 AND (?1='' OR instr(lower(coalesce(json_extract(document,'$.transcript'),'') || char(10) || coalesce(json_extract(document,'$.transformed'),'')), lower(?1))>0)";
            let total: i64 = db.query_row(&format!("SELECT count(*) FROM entries WHERE {filter}"), [&query.search], |r| r.get(0))?;
            let direction = if query.descending { "DESC" } else { "ASC" };
            let expression = format!("json_extract(document,'$.{field}')");
            let collation = if matches!(field.as_str(), "duration" | "bytes") { "" } else { "COLLATE NOCASE" };
            let mut statement = db.prepare(&format!("SELECT document FROM entries WHERE {filter} ORDER BY ({expression} IS NULL), {expression} {collation} {direction}, seq DESC LIMIT ?3 OFFSET ?2"))?;
            let entries = statement.query_map(params![query.search, query.offset, query.page_size], |r| r.get::<_, String>(0))?
                .map(|r| Ok(serde_json::from_str(&r?)?)).collect::<Result<Vec<Entry>>>()?;
            Ok(RecordingPage { entries, total: total as u64 })
        })
    }
    pub fn edit_recording(
        &self,
        id: &str,
        rev: u64,
        title: String,
        subtitle: String,
        transcript: String,
    ) -> Result<()> {
        valid_id(id)?;
        if title.trim().is_empty()
            || title.chars().count() > 500
            || subtitle.chars().count() > 2000
            || transcript.len() > 1_000_000
        {
            return Err(AppError::input("Use a title up to 500 characters, subtitle up to 2000 characters, and transcript up to 1 MB."));
        }
        let id = id.to_owned();
        self.call(move |db| {
            let tx = db.transaction()?;
            let mut entry = read_entry(&tx, &id)?;
            if entry.revision != rev { return Err(AppError::input("Entry changed. Reload before saving.")); }
            if entry.transcript.as_deref().unwrap_or("") != transcript {
                let pending: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM jobs WHERE entry_id=?1 AND status IN ('queued','running','canceling','paused','unknown'))", [&id], |r| r.get(0))?;
                if pending { return Err(AppError::input("Finish or discard pending work before editing its transcript.")); }
                entry.transcript = Some(transcript);
                entry.transcription_ms = None;
                entry.transcript_revision += 1;
                // Preserve previous output and provenance. Its revision now makes it visibly stale.
            }
            entry.title = title;
            entry.subtitle = subtitle;
            entry.subtitle_manual = true;
            write_entry(&tx, &mut entry)?;
            tx.commit()?;
            Ok(())
        })
    }
    pub fn edit(&self, id: &str, rev: u64, title: String, subtitle: String) -> Result<()> {
        if title.trim().is_empty() || title.chars().count() > 500 || subtitle.chars().count() > 2000
        {
            return Err(AppError::input(
                "Enter a title of at most 500 characters and a subtitle of at most 2000.",
            ));
        }
        let id = id.to_owned();
        self.call(move |db| {
            let mut e = read_entry(db, &id)?;
            if e.revision != rev {
                return Err(AppError::input("Entry changed. Reload before saving."));
            }
            e.title = title;
            e.subtitle = subtitle;
            e.subtitle_manual = true;
            write_entry(db, &mut e)
        })
    }
    pub fn reserve(&self, id: &str) -> Result<()> {
        let id = id.to_owned();
        self.call(move |db| {
            capacity(db, 1)?;
            db.execute("INSERT INTO reservations VALUES(?1)", [id])?;
            Ok(())
        })
    }
    pub fn release(&self, id: &str) -> Result<()> {
        let id = id.to_owned();
        self.call(move |db| {
            db.execute("DELETE FROM reservations WHERE id=?1", [id])?;
            Ok(())
        })
    }
    pub fn admit(&self, e: Entry, mut j: Job, reservation: Option<String>) -> Result<()> {
        self.call(move |db| {
            let tx = db.transaction()?;
            if let Some(id) = reservation {
                let removed = tx.execute("DELETE FROM reservations WHERE id=?1", [id])?;
                if removed != 1 {
                    return Err(AppError::input("Capture reservation was lost."));
                }
            }
            capacity(&tx, 1)?;
            tx.execute(
                "INSERT INTO entries(id,document) VALUES(?1,?2)",
                params![e.id, serde_json::to_string(&e)?],
            )?;
            insert_job(&tx, &mut j)?;
            tx.commit()?;
            Ok(())
        })
    }
    pub fn interrupted_entry(&self, e: Entry) -> Result<()> {
        self.call(move |db| {
            db.execute(
                "INSERT OR IGNORE INTO entries(id,document) VALUES(?1,?2)",
                params![e.id, serde_json::to_string(&e)?],
            )?;
            Ok(())
        })
    }
    pub fn jobs(&self) -> Result<Vec<Job>> {
        self.call(|db|{
let mut q=db.prepare("SELECT document FROM jobs WHERE status NOT IN ('complete','discarded') OR seq IN (SELECT seq FROM jobs WHERE status='complete' ORDER BY seq DESC LIMIT 100) ORDER BY seq")?;
let rows=q.query_map([],|r|r.get::<_,String>(0))?;
rows.map(|r|Ok(serde_json::from_str(&r?)?)).collect()}
)
    }
    pub fn next(&self) -> Result<Option<Job>> {
        self.call(|db| {
            let row: Option<(i64, String)> = db
                .query_row(
                    "SELECT seq,document FROM jobs WHERE status='queued' ORDER BY seq LIMIT 1",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            if let Some((seq, s)) = row {
                let mut j: Job = serde_json::from_str(&s)?;
                j.seq = seq;
                j.status = "running".into();
                write_job(db, &j)?;
                Ok(Some(j))
            } else {
                Ok(None)
            }
        })
    }
    pub fn checkpoint(&self, j: &Job) -> Result<()> {
        let j = j.clone();
        self.call(move |db| {
            ensure_not_canceled(db, &j.id)?;
            write_job(db, &j)
        })
    }
    /// Includes capture/import reservations; callers serialize admission with the native gate.
    pub fn processing_busy(&self) -> Result<bool> {
        self.call(|db| Ok(db.query_row("SELECT EXISTS(SELECT 1 FROM jobs WHERE status IN ('queued','running','canceling')) OR EXISTS(SELECT 1 FROM reservations)", [], |r| r.get(0))?))
    }
    pub fn ensure_processing_idle(&self) -> Result<()> {
        if self.processing_busy()? {
            return Err(AppError::input(
                "Finish or cancel the current operation first.",
            ));
        }
        Ok(())
    }
    pub fn cancel_processing(&self) -> Result<()> {
        self.call(|db| {
            let tx = db.transaction()?;
            let jobs = {
                let mut q =
                    tx.prepare("SELECT document FROM jobs WHERE status IN ('queued','running')")?;
                let rows = q.query_map([], |r| r.get::<_, String>(0))?;
                rows.collect::<std::result::Result<Vec<_>, _>>()?
            };
            for value in jobs {
                let mut j: Job = serde_json::from_str(&value)?;
                j.status = if j.status == "running" {
                    "canceling"
                } else {
                    "canceled"
                }
                .into();
                j.eligible = false;
                j.error = None;
                write_job(&tx, &j)?;
            }
            tx.commit()?;
            Ok(())
        })
    }
    pub fn cancellation_requested(&self, id: &str) -> Result<bool> {
        let id = id.to_owned();
        self.call(move |db| {
            Ok(db.query_row(
                "SELECT status IN ('canceling','canceled') FROM jobs WHERE id=?1",
                [id],
                |r| r.get(0),
            )?)
        })
    }
    pub fn finish_cancellation(&self, id: &str) -> Result<()> {
        let id = id.to_owned();
        self.call(move |db| {
            let value: String =
                db.query_row("SELECT document FROM jobs WHERE id=?1", [&id], |r| r.get(0))?;
            let mut j: Job = serde_json::from_str(&value)?;
            if j.status == "canceling" {
                j.status = "canceled".into();
                write_job(db, &j)?;
            }
            Ok(())
        })
    }
    pub fn transcript(&self, j: &Job, text: String, elapsed_ms: Option<u64>) -> Result<()> {
        let j = j.clone();
        self.call(move |db| {
            let tx = db.transaction()?;
            ensure_not_canceled(&tx, &j.id)?;
            let mut e = read_entry(&tx, &j.entry_id)?;
            e.transcript = Some(text);
            e.transcription_ms = elapsed_ms;
            e.transcript_revision += 1;
            e.transcript_model = Some(format!(
                "{} / {}",
                j.snapshot.transcription.provider,
                if j.snapshot.transcription_mode == TranscriptionMode::Realtime
                    && j.kind == "initial"
                {
                    crate::realtime::MODEL
                } else {
                    &j.snapshot.transcription.model
                }
            ));
            write_entry(&tx, &mut e)?;
            write_job(&tx, &j)?;
            tx.commit()?;
            Ok(())
        })
    }
    pub fn transformed(&self, j: &Job, text: String, elapsed_ms: Option<u64>) -> Result<()> {
        let j = j.clone();
        self.call(move |db| {
            let tx = db.transaction()?;
            ensure_not_canceled(&tx, &j.id)?;
            let mut e = read_entry(&tx, &j.entry_id)?;
            if e.transcript_revision != j.input_revision {
                return Err(AppError::input(
                    "Transcript changed while transformation was pending.",
                ));
            }
            e.transformed = Some(text);
            e.transformation_ms = elapsed_ms;
            e.transformed_revision = Some(j.input_revision);
            e.transform_model = j
                .snapshot
                .preset
                .as_ref()
                .map(|p| format!("{} / {}", p.provider, p.model));
            write_entry(&tx, &mut e)?;
            write_job(&tx, &j)?;
            tx.commit()?;
            Ok(())
        })
    }
    pub fn retry(&self, id: &str, rev: u64, kind: &str, snap: Snapshot) -> Result<()> {
        let id = id.to_owned();
        let kind = kind.to_owned();
        self.call(move|db|{
let tx=db.transaction()?;
let e=read_entry(&tx,&id)?;
if e.revision!=rev{
return Err(AppError::input("Entry changed. Reload before retrying."));
}
if !["transcribe","transform"].contains(&kind.as_str()){
return Err(AppError::input("Invalid retry stage."));
}
if kind=="transcribe"&&e.audio.is_none(){
return Err(AppError::input("Original audio is no longer retained."));
}
if kind=="transform"&&(e.transcript.is_none()||snap.preset.is_none()){
return Err(AppError::input("A transcript and selected transformation preset are required."));
}
  let pending:Option<(String,String)>=tx.query_row("SELECT id,status FROM jobs WHERE entry_id=?1 AND status IN ('queued','running','canceling','paused','unknown')",[&id],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
  if let Some((pending,status))=pending {
if status!="unknown"{
return Err(AppError::input("This entry already has pending work. Resume or discard paused work first."));
}
let s:String=tx.query_row("SELECT document FROM jobs WHERE id=?1",[&pending],|r|r.get(0))?;
let mut old:Job=serde_json::from_str(&s)?;
old.status="discarded".into();
write_job(&tx,&old)?;
}
  capacity(&tx,1)?;
let mut j=Job::new(id,&kind,snap,false);
if kind=="transform"{
j.input=e.transcript;
j.input_revision=e.transcript_revision;
}
insert_job(&tx,&mut j)?;
tx.commit()?;
Ok(())}
)
    }
    pub fn resume(&self, ids: Vec<String>, discard: bool) -> Result<()> {
        self.call(move|db|{
let tx=db.transaction()?;
let mut selected=Vec::new();
{
let mut q=tx.prepare("SELECT document FROM jobs WHERE status IN ('paused','unknown') ORDER BY seq")?;
for row in q.query_map([],|r|r.get::<_,String>(0))?{
let j:Job=serde_json::from_str(&row?)?;
if ids.contains(&j.id){
selected.push(j);
}
}
}
if selected.len()!=ids.len(){
return Err(AppError::input("Paused work changed. Reload before continuing."));
}
if !discard{
capacity(&tx,selected.len())?;
if selected.iter().any(|j|j.status=="unknown" || j.requires_explicit_retry()){
return Err(AppError::new("recovery","unknown_outcome","A provider response was lost. Explicitly retry the affected stage; another charge may apply."));
}
}
  for mut j in selected{
j.eligible=false;
if discard{
j.status="discarded".into();
write_job(&tx,&j)?;
}
else{
tx.execute("DELETE FROM jobs WHERE id=?1",[&j.id])?;
j.status="queued".into();
j.error=None;
insert_job(&tx,&mut j)?;
}
}
tx.commit()?;
Ok(())}
)
    }
    fn recover(&self) -> Result<()> {
        self.call(|db| {
            let mut list = Vec::new();
            {
                let mut q =
                    db.prepare("SELECT document FROM jobs WHERE status IN ('queued','running','canceling','paused','complete')")?;
                for row in q.query_map([], |r| r.get::<_, String>(0))? {
                    list.push(serde_json::from_str::<Job>(&row?)?);
                }
            }
            for mut j in list {
                if j.status == "canceling" { j.status = "canceled".into(); write_job(db, &j)?; continue; }
                if j.status == "complete" {
                    if j.error.take().is_some() { write_job(db, &j)?; }
                    continue;
                }
                j.eligible = false;
                j.status = if j.requires_explicit_retry() {
                    "unknown"
                } else {
                    "paused"
                }
                .into();
                if j.stage == "ready" {
                    j.delivery = "manual_only".into();
                }
                write_job(db, &j)?;
            }
            Ok(())
        })
    }
    pub fn pin_audio(&self, id: &str) -> Result<Entry> {
        let id = id.to_owned();
        self.call(move|db|{let e=read_entry(db,&id)?;if e.audio.is_none(){return Err(AppError::input("Original audio is no longer retained."));}db.execute("INSERT INTO audio_leases(entry_id) VALUES(?1) ON CONFLICT(entry_id) DO UPDATE SET leases=leases+1",[id])?;Ok(e)})
    }
    pub fn lease(&self, id: &str, on: bool) -> Result<()> {
        if on {
            self.pin_audio(id).map(|_| ())
        } else {
            let id = id.to_owned();
            self.call(move |db| {
                db.execute(
                    "UPDATE audio_leases SET leases=leases-1 WHERE entry_id=?1",
                    [&id],
                )?;
                db.execute(
                    "DELETE FROM audio_leases WHERE entry_id=?1 AND leases<=0",
                    [id],
                )?;
                Ok(())
            })
        }
    }
    fn deletion_finished(&self, id: &str) -> Result<()> {
        let id = id.to_owned();
        let pending: bool = self.call(move |db| {
            Ok(db.query_row(
                "SELECT EXISTS(SELECT 1 FROM entries WHERE id=?1 AND deleting=1)",
                [id],
                |r| r.get(0),
            )?)
        })?;
        if pending {
            return Err(AppError::new("storage","unlink","Deletion is pending because the audio file is in use or inaccessible. Retry deletion or reopen x-talk to finish cleanup."));
        }
        Ok(())
    }
    pub fn delete(&self, id: &str, rev: u64, confirmed: bool) -> Result<()> {
        if !confirmed {
            return Err(AppError::input("Confirm entry deletion first."));
        }
        valid_id(id)?;
        let pending_id = id.to_owned();
        let pending: bool = self.call(move |db| {
            Ok(db.query_row(
                "SELECT EXISTS(SELECT 1 FROM entries WHERE id=?1 AND deleting=1)",
                [pending_id],
                |r| r.get(0),
            )?)
        })?;
        if pending {
            self.reconcile()?;
            return self.deletion_finished(id);
        }
        let verify_id = id.to_owned();
        let id = id.to_owned();
        self.call(move|db|{
let tx=db.transaction()?;
let e=read_entry(&tx,&id)?;
if e.revision!=rev{
return Err(AppError::input("Entry changed. Reload before deleting."));
}
let busy:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM jobs WHERE entry_id=?1 AND status IN ('queued','running','canceling')) OR EXISTS(SELECT 1 FROM audio_leases WHERE entry_id=?1)",[&id],|r|r.get(0))?;
if busy{
return Err(AppError::input("Wait for processing or playback to finish before deleting."));
}
if let Some(path)=e.audio{
tx.execute("INSERT OR IGNORE INTO file_gc VALUES(?1)",[path])?;
}
tx.execute("UPDATE entries SET deleting=1 WHERE id=?1",[id])?;
tx.commit()?;
Ok(())}
)?;
        self.reconcile()?;
        self.deletion_finished(&verify_id)
    }
    pub fn update_busy(&self) -> Result<bool> {
        self.call(|db| Ok(db.query_row(
            "SELECT EXISTS(SELECT 1 FROM jobs WHERE status IN ('queued','running','canceling')) OR EXISTS(SELECT 1 FROM audio_leases) OR EXISTS(SELECT 1 FROM reservations)",
            [], |r| r.get(0),
        )?))
    }
    pub fn clear_recordings(&self, confirmed: bool) -> Result<u64> {
        if !confirmed {
            return Err(AppError::input("Confirm clearing all recordings first."));
        }
        let count = self.call(|db| {
            let tx = db.transaction()?;
            let busy: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM jobs WHERE status IN ('queued','running','canceling')) OR EXISTS(SELECT 1 FROM audio_leases) OR EXISTS(SELECT 1 FROM reservations)",
                [], |r| r.get(0),
            )?;
            if busy {
                return Err(AppError::input("Wait for processing, imports, and audio access to finish before clearing the library."));
            }
            let count: i64 = tx.query_row("SELECT count(*) FROM entries WHERE deleting=0", [], |r| r.get(0))?;
            tx.execute(
                "INSERT OR IGNORE INTO file_gc(path) SELECT json_extract(document,'$.audio') FROM entries WHERE json_extract(document,'$.audio') IS NOT NULL", [],
            )?;
            tx.execute("UPDATE entries SET deleting=1", [])?;
            tx.execute("DELETE FROM jobs", [])?;
            tx.commit()?;
            Ok(count as u64)
        })?;
        self.reconcile()?;
        let pending: bool = self.call(|db| {
            Ok(db.query_row(
                "SELECT EXISTS(SELECT 1 FROM entries WHERE deleting=1)",
                [],
                |r| r.get(0),
            )?)
        })?;
        if pending {
            return Err(AppError::new("storage", "unlink", "Some audio files are still in use or inaccessible. Retry Clear All or reopen x-talk to finish cleanup."));
        }
        Ok(count)
    }
    pub fn retention(&self) -> Result<()> {
        let keep = self.settings()?.keep_audio;
        if let Some(keep) = keep {
            self.call(move|db|{
let tx=db.transaction()?;
let mut list=Vec::new();
{
let mut q=tx.prepare("SELECT document FROM entries WHERE deleting=0 ORDER BY seq DESC LIMIT -1 OFFSET ?1")?;
for row in q.query_map([keep],|r|r.get::<_,String>(0))?{
list.push(serde_json::from_str::<Entry>(&row?)?);
}
}
for mut e in list{
let pinned:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM jobs WHERE entry_id=?1 AND status IN ('queued','running','canceling','paused','unknown','canceled')) OR EXISTS(SELECT 1 FROM audio_leases WHERE entry_id=?1)",[&e.id],|r|r.get(0))?;
if !pinned{
if let Some(path)=e.audio.take(){
tx.execute("INSERT OR IGNORE INTO file_gc VALUES(?1)",[path])?;
write_entry(&tx,&mut e)?;
}
}
}
tx.commit()?;
Ok(())}
)?;
        }
        self.reconcile()
    }
    pub fn path(&self, relative: &str) -> Result<PathBuf> {
        let p = Path::new(relative);
        if p.components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
            || !relative.starts_with("audio/")
        {
            return Err(AppError::input("Invalid managed audio path."));
        }
        Ok(self.root.join(p))
    }
    pub fn reconcile(&self) -> Result<()> {
        let gc: Vec<String> = self.call(|db| {
            let mut q = db.prepare("SELECT path FROM file_gc")?;
            let v = q
                .query_map([], |r| r.get(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(v)
        })?;
        for path in gc {
            let full = self.path(&path)?;
            match std::fs::remove_file(full) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => continue,
            }
            self.call(move |db| {
                db.execute("DELETE FROM file_gc WHERE path=?1", [path])?;
                Ok(())
            })?;
        }
        self.call(|db|{
db.execute("DELETE FROM entries WHERE deleting=1 AND NOT EXISTS(SELECT 1 FROM file_gc WHERE path=json_extract(entries.document,'$.audio'))",[])?;
Ok(())}
)?;
        let root = self.root.clone();
        self.call(move |db| {
            let mut all = Vec::new();
            {
                let mut q = db.prepare("SELECT document FROM entries WHERE deleting=0")?;
                for row in q.query_map([], |r| r.get::<_, String>(0))? {
                    all.push(serde_json::from_str::<Entry>(&row?)?);
                }
            }
            for mut e in all {
                if e.audio.as_ref().is_some_and(|p| !root.join(p).is_file()) {
                    e.audio = None;
                    write_entry(db, &mut e)?;
                }
            }
            Ok(())
        })
    }
}
fn capacity(db: &Connection, add: usize) -> Result<()> {
    let count:i64=db.query_row("SELECT (SELECT count(*) FROM jobs WHERE status IN ('queued','running','canceling'))+(SELECT count(*) FROM reservations)",[],|r|r.get(0))?;
    if count as usize + add > CAPACITY {
        return Err(AppError::input(format!(
            "Queue full. {} active slots are available.",
            CAPACITY.saturating_sub(count as usize)
        )));
    }
    Ok(())
}
fn read_entry(db: &Connection, id: &str) -> Result<Entry> {
    let s: String = db
        .query_row(
            "SELECT document FROM entries WHERE id=?1 AND deleting=0",
            [id],
            |r| r.get(0),
        )
        .optional()?
        .ok_or_else(|| AppError::input("Entry no longer exists."))?;
    Ok(serde_json::from_str(&s)?)
}
fn write_entry(db: &Connection, e: &mut Entry) -> Result<()> {
    e.revision += 1;
    db.execute(
        "UPDATE entries SET document=?1 WHERE id=?2",
        params![serde_json::to_string(e)?, e.id],
    )?;
    Ok(())
}
fn ensure_not_canceled(db: &Connection, id: &str) -> Result<()> {
    let canceled: bool = db.query_row(
        "SELECT status IN ('canceling','canceled') FROM jobs WHERE id=?1",
        [id],
        |r| r.get(0),
    )?;
    if canceled {
        return Err(AppError::new(
            "processing",
            "canceled",
            "Processing canceled. Saved audio and text are preserved.",
        ));
    }
    Ok(())
}
fn write_job(db: &Connection, j: &Job) -> Result<()> {
    db.execute(
        "UPDATE jobs SET status=?1,stage=?2,document=?3 WHERE id=?4",
        params![j.status, j.stage, serde_json::to_string(j)?, j.id],
    )?;
    Ok(())
}
fn insert_job(db: &Connection, j: &mut Job) -> Result<()> {
    db.execute(
        "INSERT INTO jobs(id,entry_id,status,stage,document) VALUES(?1,?2,?3,?4,?5)",
        params![
            j.id,
            j.entry_id,
            j.status,
            j.stage,
            serde_json::to_string(j)?
        ],
    )?;
    j.seq = db.last_insert_rowid();
    write_job(db, j)
}

/// Publish a complete, synced journal without exposing a partially written final file.
pub(crate) fn publish_journal(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let mut temporary = path.as_os_str().to_os_string();
    temporary.push(".new");
    let temporary = PathBuf::from(temporary);
    let mut file = std::fs::File::create(&temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{
            MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
        };
        let source: Vec<u16> = temporary.as_os_str().encode_wide().chain(Some(0)).collect();
        let target: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        if unsafe {
            MoveFileExW(
                source.as_ptr(),
                target.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    #[cfg(not(windows))]
    {
        std::fs::rename(&temporary, path)?;
        if let Some(parent) = path.parent() {
            std::fs::File::open(parent)?.sync_all()?;
        }
    }
    Ok(())
}

fn read_active_preset(db: &Connection) -> Result<Preset> {
    let document: String =
        db.query_row("SELECT document FROM settings WHERE id=1", [], |r| r.get(0))?;
    let settings: Settings = serde_json::from_str(&document)?;
    let document: String = db.query_row(
        "SELECT document FROM presets WHERE id=?1",
        [settings.preset_id],
        |r| r.get(0),
    )?;
    Ok(serde_json::from_str(&document)?)
}
