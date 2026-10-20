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
fn write(db: &Connection, meeting: &Meeting) -> Result<()> {
    db.execute(
        "UPDATE meetings SET revision=?2, document=?3 WHERE id=?1",
        params![
            meeting.id,
            i64::try_from(meeting.revision)
                .map_err(|_| AppError::input("Meeting revision overflow."))?,
            serde_json::to_string(meeting)?
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
        db.execute("DELETE FROM audio_leases", [])?;
        for mut m in list(&db)? {
            let old_stage = m.attempt.stage.clone();
            m.attempt.recovery();
            if m.attempt.stage != old_stage {
                if m.brief.status == Stage::Briefing {
                    m.brief.status = Stage::SubmissionUnknown;
                }
                m.revision += 1;
                write(&db, &m)?;
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
    pub fn pending(&self) -> Result<Vec<Meeting>> {
        self.call(|db| {
            let mut stmt = db.prepare("SELECT document FROM meetings WHERE json_extract(document,'$.attempt.stage') IN ('queued','waiting_for_keys','polling') ORDER BY added_at")?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
            rows.map(|row| Ok(serde_json::from_str(&row?)?)).collect()
        })
    }
    pub fn update_busy(&self) -> Result<bool> {
        self.call(|db| Ok(db.query_row(
            "SELECT EXISTS(SELECT 1 FROM audio_leases) OR EXISTS(SELECT 1 FROM meetings WHERE json_extract(document,'$.attempt.stage') IN ('queued','preparing','submitting','polling','briefing'))",
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
            if meeting.audio_hash.is_some() && list(db)?.iter().any(|m| m.audio_hash == meeting.audio_hash) {
                return Err(AppError::input("This audio is already in Meetings. Open the existing meeting to review or re-transcribe it."));
            }
            db.execute(
                "INSERT INTO meetings VALUES(?1,?2,?3,?4)",
                params![
                    meeting.id,
                    meeting.added_at,
                    i64::try_from(meeting.revision)
                        .map_err(|_| AppError::input("Meeting revision overflow."))?,
                    serde_json::to_string(&meeting)?
                ],
            )?;
            Ok(())
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
                for (index, mut m) in list(&tx)?
                    .into_iter()
                    .filter(|m| m.audio.is_some())
                    .enumerate()
                {
                    if index < keep as usize || m.attempt.protects_audio() {
                        continue;
                    }
                    let leased: bool = tx.query_row(
                        "SELECT EXISTS(SELECT 1 FROM audio_leases WHERE id=?1)",
                        [&m.id],
                        |r| r.get(0),
                    )?;
                    if leased {
                        continue;
                    }
                    tx.execute("INSERT OR IGNORE INTO file_gc VALUES(?1)", [&m.id])?;
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
