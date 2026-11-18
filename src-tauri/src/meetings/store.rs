use super::domain::*;
use crate::domain::{valid_id, AppError, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::{
    path::{Path, PathBuf},
    sync::{mpsc, Arc, Mutex},
};

type Work = Box<dyn FnOnce(&mut Connection) + Send>;
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
#[derive(Clone)]
pub struct MeetingStore {
    worker: Arc<DatabaseWorker>,
    pub root: Arc<PathBuf>,
}
pub struct AudioLease {
    store: MeetingStore,
    id: String,
}
impl Drop for AudioLease {
    fn drop(&mut self) {
        let id = self.id.clone();
        let _ = self.store.call(move |db| {
            db.execute("UPDATE audio_leases SET count=count-1 WHERE id=?1", [&id])?;
            db.execute("DELETE FROM audio_leases WHERE count<=0", [])?;
            Ok(())
        });
        let _ = self.store.prune();
    }
}

fn read(db: &Connection, id: &str) -> Result<Meeting> {
    let document: Option<String> = db
        .query_row("SELECT document FROM meetings WHERE id=?1", [id], |r| {
            r.get(0)
        })
        .optional()?;
    Ok(serde_json::from_str(&document.ok_or_else(|| {
        AppError::input("This meeting was deleted or does not exist.")
    })?)?)
}
fn document(meeting: &Meeting) -> Result<String> {
    let mut document = serde_json::to_string(meeting)?;
    // Derived list metadata is ignored when deserializing the full domain record.
    // Build both from the same revision, without another transcript-sized Value tree.
    document.truncate(document.len() - 1);
    document.push_str(",\"_list\":");
    document.push_str(&serde_json::to_string(&Summary::from_meeting(meeting)?)?);
    document.push('}');
    Ok(document)
}
fn write(db: &Connection, meeting: &Meeting) -> Result<()> {
    db.execute(
        "UPDATE meetings SET revision=?2, document=?3 WHERE id=?1",
        params![
            meeting.id,
            i64::try_from(meeting.revision)
                .map_err(|_| AppError::input("Meeting revision overflow."))?,
            document(meeting)?
        ],
    )?;
    Ok(())
}
fn list(db: &Connection) -> Result<Vec<Meeting>> {
    let mut stmt = db.prepare("SELECT document FROM meetings ORDER BY added_at DESC,id DESC")?;
    let docs = stmt.query_map([], |r| r.get::<_, String>(0))?;
    docs.map(|d| Ok(serde_json::from_str(&d?)?)).collect()
}
impl MeetingStore {
    pub fn open(root: &Path) -> Result<Self> {
        std::fs::create_dir_all(root.join("audio"))?;
        std::fs::create_dir_all(root.join("staging"))?;
        let mut db = Connection::open(root.join("meetings.sqlite3"))?;
        db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA busy_timeout=5000;")?;
        let version: u32 = db.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version > 1 {
            return Err(AppError::input(
                "Meeting data requires a newer xTalk version.",
            ));
        }
        if version == 0 {
            let tx = db.transaction()?;
            tx.execute_batch(include_str!("../../migrations/meetings/001.sql"))?;
            tx.commit()?;
        }
        // Expression indexes keep periodic jobs and library reads out of old
        // transcript payloads. Safe additive upgrade for earlier local previews.
        db.execute_batch("CREATE INDEX IF NOT EXISTS meetings_stage ON meetings(json_extract(document,'$.attempt.stage'),added_at);
            CREATE INDEX IF NOT EXISTS meetings_audio_hash ON meetings(json_extract(document,'$.audio_hash')) WHERE json_extract(document,'$.audio_hash') IS NOT NULL;
            CREATE INDEX IF NOT EXISTS meetings_list_summary ON meetings(added_at DESC,id DESC,json_extract(document,'$._list'));
            CREATE INDEX IF NOT EXISTS meetings_retained ON meetings(added_at DESC,id DESC,json_extract(document,'$.attempt.stage')) WHERE json_extract(document,'$.audio') IS NOT NULL;")?;
        db.execute("DELETE FROM audio_leases", [])?;
        let recoveries = {
            let mut statement = db.prepare("SELECT document FROM meetings WHERE json_extract(document,'$.attempt.stage') IN ('submitting','preparing','briefing','importing','extracting','converting')")?;
            let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
            rows.map(|row| Ok(serde_json::from_str::<Meeting>(&row?)?))
                .collect::<Result<Vec<_>>>()?
        };
        for mut m in recoveries {
            let old_stage = m.attempt.stage.clone();
            m.attempt.recovery();
            if m.attempt.stage != old_stage {
                if m.brief.status == Stage::Briefing {
                    m.brief.status = Stage::SubmissionUnknown;
                }
                m.revision += 1;
                write(&db, &m)?;
                if old_stage.importing() {
                    let _ = std::fs::remove_dir_all(root.join("audio").join(&m.id));
                }
            }
        }
        let (tx, rx) = mpsc::channel::<Work>();
        let thread = std::thread::Builder::new()
            .name("meetings-database".into())
            .spawn(move || {
                for work in rx {
                    work(&mut db);
                }
            })?;
        let store = Self {
            root: Arc::new(root.into()),
            worker: Arc::new(DatabaseWorker {
                tx: Some(tx),
                thread: Mutex::new(Some(thread)),
            }),
        };
        store.collect_garbage()?;
        Ok(store)
    }
    fn call<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let (tx, rx) = mpsc::sync_channel(1);
        self.worker
            .tx
            .as_ref()
            .ok_or_else(|| AppError::input("Meeting storage closed."))?
            .send(Box::new(move |db| {
                let _ = tx.send(f(db));
            }))
            .map_err(|_| AppError::input("Meeting storage closed."))?;
        rx.recv()
            .map_err(|_| AppError::input("Meeting storage closed."))?
    }
    pub fn get(&self, id: &str) -> Result<Meeting> {
        valid_id(id)?;
        let id = id.to_string();
        self.call(move |db| read(db, &id))
    }
    pub fn list(&self) -> Result<Vec<Meeting>> {
        self.call(|db| list(db))
    }
    pub fn summaries(&self) -> Result<Vec<Summary>> {
        self.call(|db| {
            let rows = {
                let mut statement = db.prepare("SELECT id,json_extract(document,'$._list') FROM meetings ORDER BY added_at DESC,id DESC")?;
                let rows = statement.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)))?;
                rows.collect::<std::result::Result<Vec<_>, _>>()?
            };
            rows.into_iter().map(|(id, cached)| {
                if let Some(summary) = cached.and_then(|json| serde_json::from_str::<Summary>(&json).ok()) { return Ok(summary); }
                // Older previews are upgraded lazily, one document at a time.
                let meeting = read(db, &id)?;
                let summary = Summary::from_meeting(&meeting)?;
                write(db, &meeting)?;
                Ok(summary)
            }).collect()
        })
    }
    pub fn pending(&self) -> Result<Vec<Meeting>> {
        self.call(|db| {
            let mut stmt = db.prepare("SELECT document FROM meetings WHERE json_extract(document,'$.attempt.stage') IN ('queued','waiting_for_keys','polling') ORDER BY added_at")?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
            rows.map(|row| Ok(serde_json::from_str(&row?)?)).collect()
        })
    }
    pub fn update_busy(&self) -> Result<bool> {
        self.call(|db| Ok(db.query_row(
            "SELECT EXISTS(SELECT 1 FROM audio_leases) OR EXISTS(SELECT 1 FROM meetings WHERE json_extract(document,'$.attempt.stage') IN ('importing','extracting','converting','queued','preparing','submitting','polling','briefing'))",
            [], |r| r.get(0))?))
    }
    pub fn recoverable(&self, id: &str) -> Result<bool> {
        valid_id(id)?;
        let id = id.to_string();
        self.call(move |db| Ok(!db.query_row(
            "SELECT EXISTS(SELECT 1 FROM meetings WHERE id=?1) OR EXISTS(SELECT 1 FROM file_gc WHERE id=?1)",
            [id], |r| r.get::<_, bool>(0))?))
    }
    pub fn insert(&self, meeting: Meeting) -> Result<()> {
        valid_id(&meeting.id)?;
        if let Some(audio) = &meeting.audio {
            for asset in &audio.assets {
                let path = self.audio_path(&meeting.id, asset)?;
                if !path.is_file() {
                    return Err(AppError::input("Meeting audio has not been saved."));
                }
                std::fs::OpenOptions::new()
                    .write(true)
                    .open(path)?
                    .sync_all()?;
            }
            crate::storage::publish_journal(
                &self
                    .root
                    .join("audio")
                    .join(&meeting.id)
                    .join("admission.json"),
                &serde_json::to_vec(&meeting)?,
            )?;
        }
        self.call(move |db| {
            if let Some(hash) = &meeting.audio_hash {
                let duplicate: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM meetings WHERE json_extract(document,'$.audio_hash')=?1)", [hash], |row| row.get(0))?;
                if duplicate { return Err(AppError::input("This audio is already in Meetings. Open the existing meeting to review or re-transcribe it.")); }
            }
            db.execute(
                "INSERT INTO meetings VALUES(?1,?2,?3,?4)",
                params![
                    meeting.id,
                    meeting.added_at,
                    i64::try_from(meeting.revision)
                        .map_err(|_| AppError::input("Meeting revision overflow."))?,
                    document(&meeting)?
                ],
            )?;
            Ok(())
        })
    }
    /// Publish prepared assets and duplicate detection in one serialized transaction.
    pub fn complete_import(
        &self,
        id: &str,
        attempt: &str,
        audio: Audio,
        hash: String,
        duration: f64,
    ) -> Result<Meeting> {
        let id = id.to_owned();
        let attempt = attempt.to_owned();
        self.call(move |db| {
            let tx=db.transaction()?;
            let mut m=read(&tx,&id)?;
            if m.attempt.id != attempt || !m.attempt.stage.importing() { return Err(AppError::input("This import is no longer active.")); }
            let duplicate:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM meetings WHERE id<>?1 AND json_extract(document,'$.audio_hash')=?2)",params![id,hash],|r|r.get(0))?;
            if duplicate { return Err(AppError::input("This audio is already in Meetings. Open the existing meeting to review or re-transcribe it.")); }
            m.duration=Some(duration); m.audio=Some(audio); m.audio_hash=Some(hash); m.attempt.stage=Stage::Queued; m.attempt.error=None; m.revision+=1;
            write(&tx,&m)?; tx.commit()?; Ok(m)
        })
    }
    /// Revision checked UI edits cannot overwrite a newer provider publication.
    pub fn edit(
        &self,
        id: &str,
        revision: u64,
        f: impl FnOnce(&mut Meeting) -> Result<()> + Send + 'static,
    ) -> Result<Meeting> {
        let id = id.to_string();
        self.call(move |db| {
            let tx = db.transaction()?;
            let mut m = read(&tx, &id)?;
            if m.revision != revision {
                return Err(AppError::input(
                    "This meeting changed. Refresh and try again.",
                ));
            }
            f(&mut m)?;
            m.revision += 1;
            write(&tx, &m)?;
            tx.commit()?;
            Ok(m)
        })
    }
    /// Jobs publish into the latest document, preserving concurrent metadata edits.
    pub fn publish(
        &self,
        id: &str,
        attempt: &str,
        f: impl FnOnce(&mut Meeting) -> Result<()> + Send + 'static,
    ) -> Result<Meeting> {
        let id = id.to_string();
        let attempt = attempt.to_string();
        self.call(move |db| {
            let tx = db.transaction()?;
            let mut m = read(&tx, &id)?;
            if m.attempt.id != attempt {
                return Err(AppError::input("This meeting attempt was superseded."));
            }
            f(&mut m)?;
            m.revision += 1;
            write(&tx, &m)?;
            tx.commit()?;
            Ok(m)
        })
    }
    pub fn audio_path(&self, id: &str, asset: &str) -> Result<PathBuf> {
        valid_id(id)?;
        if asset.is_empty()
            || asset == "."
            || asset == ".."
            || asset
                .chars()
                .any(|c| !c.is_ascii_alphanumeric() && c != '.' && c != '-' && c != '_')
        {
            return Err(AppError::input("Invalid meeting audio asset."));
        }
        Ok(self.root.join("audio").join(id).join(asset))
    }
    pub fn lease(&self, id: &str) -> Result<AudioLease> {
        let key = id.to_string();
        self.call(move |db| {
            let tx = db.transaction()?;
            if read(&tx, &key)?.audio.is_none() {
                return Err(AppError::input(
                    "This meeting's audio is no longer retained.",
                ));
            }
            tx.execute(
                "INSERT INTO audio_leases VALUES(?1,1) ON CONFLICT(id) DO UPDATE SET count=count+1",
                [&key],
            )?;
            tx.commit()?;
            Ok(())
        })?;
        Ok(AudioLease {
            store: self.clone(),
            id: id.into(),
        })
    }
    /// Remove raw capture scratch only after compact source assets are admitted.
    /// Repeating this after restart closes the commit/cleanup crash window.
    pub fn cleanup_capture_working_files(&self, id: &str) -> Result<()> {
        let meeting = self.get(id)?;
        let Some(audio) = meeting.audio else {
            return Ok(());
        };
        if meeting.source != SourceKind::Recording
            || !audio.assets.iter().any(|name| name == "prepared.m4a")
            || audio.assets.iter().any(|name| name.ends_with(".f32"))
            || !audio
                .assets
                .iter()
                .any(|name| name == "microphone.m4a" || name == "system.m4a")
        {
            return Ok(());
        }
        for asset in &audio.assets {
            if !self.audio_path(id, asset)?.is_file() {
                return Ok(());
            }
        }
        for (raw, compact) in [
            ("microphone.f32", "microphone.m4a"),
            ("system.f32", "system.m4a"),
        ] {
            if self.audio_path(id, raw)?.exists()
                && !audio.assets.iter().any(|name| name == compact)
            {
                return Ok(());
            }
        }
        for filename in ["microphone.f32", "system.f32"] {
            match std::fs::remove_file(self.audio_path(id, filename)?) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }
    pub fn retention(&self) -> Result<Option<u32>> {
        self.call(|db| {
            Ok(
                db.query_row("SELECT retain_count FROM settings WHERE id=1", [], |r| {
                    r.get(0)
                })?,
            )
        })
    }
    pub fn set_retention(&self, count: Option<u32>) -> Result<()> {
        if count == Some(0) {
            return Err(AppError::input(
                "Retain at least one meeting, or choose unlimited.",
            ));
        }
        self.call(move |db| {
            db.execute("UPDATE settings SET retain_count=?1 WHERE id=1", [count])?;
            Ok(())
        })?;
        self.prune()
    }
    pub fn prune(&self) -> Result<()> {
        self.call(|db| {
            let tx = db.transaction()?;
            let keep: Option<u32> =
                tx.query_row("SELECT retain_count FROM settings WHERE id=1", [], |r| {
                    r.get(0)
                })?;
            if let Some(keep) = keep {
                let candidates = {
                    let mut statement = tx.prepare("SELECT id,json_extract(document,'$.attempt.stage') FROM meetings WHERE json_extract(document,'$.audio') IS NOT NULL ORDER BY added_at DESC,id DESC")?;
                    let rows = statement.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?;
                    rows.collect::<std::result::Result<Vec<_>, _>>()?
                };
                for (index, (id, stage)) in candidates.into_iter().enumerate() {
                    if index < keep as usize || stage != "complete" { continue; }
                    let leased: bool = tx.query_row(
                        "SELECT EXISTS(SELECT 1 FROM audio_leases WHERE id=?1)",
                        [&id],
                        |r| r.get(0),
                    )?;
                    if leased {
                        continue;
                    }
                    tx.execute("INSERT OR IGNORE INTO file_gc VALUES(?1)", [&id])?;
                    let mut m = read(&tx, &id)?;
                    m.audio = None;
                    m.revision += 1;
                    write(&tx, &m)?;
                }
            }
            tx.commit()?;
            Ok(())
        })?;
        self.collect_garbage()
    }
    pub fn delete(&self, id: &str, revision: u64) -> Result<()> {
        let id = id.to_string();
        self.call(move |db| {
            let tx = db.transaction()?;
            let m = read(&tx, &id)?;
            if m.revision != revision {
                return Err(AppError::input(
                    "This meeting changed. Refresh before deleting.",
                ));
            }
            if m.attempt.stage.importing() {
                return Err(AppError::input(
                    "Wait for audio preparation to finish before deleting this meeting.",
                ));
            }
            let leased: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM audio_leases WHERE id=?1)",
                [&id],
                |r| r.get(0),
            )?;
            if leased {
                return Err(AppError::input(
                    "Stop playback or wait for processing before deleting this meeting.",
                ));
            }
            tx.execute("INSERT OR IGNORE INTO file_gc VALUES(?1)", [&id])?;
            tx.execute("DELETE FROM meetings WHERE id=?1", [&id])?;
            tx.commit()?;
            Ok(())
        })?;
        self.collect_garbage()
    }
    pub fn collect_garbage(&self) -> Result<()> {
        let ids: Vec<String> = self.call(|db| {
            let mut stmt = db.prepare("SELECT id FROM file_gc")?;
            let rows = stmt.query_map([], |r| r.get(0))?;
            Ok(rows.collect::<std::result::Result<_, _>>()?)
        })?;
        for id in ids {
            valid_id(&id)?;
            let path = self.root.join("audio").join(&id);
            match std::fs::remove_dir_all(path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => continue, // Keep the durable intent for the next reconciliation.
            }
            self.call(move |db| {
                db.execute("DELETE FROM file_gc WHERE id=?1", [id])?;
                Ok(())
            })?;
        }
        Ok(())
    }
}
