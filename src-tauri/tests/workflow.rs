use async_trait::async_trait;
use std::{
    path::Path,
    sync::{Arc, Mutex},
};
use x_talk::{
    domain::*,
    jobs::{ClipboardWriter, Feedback, Worker},
    media::{AudioInfo, MediaProcessor, PreparedAudio},
    providers::{TranscriptionProvider, TransformationProvider},
    storage::Store,
};
fn store() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let s = Store::open(dir.path()).unwrap();
    (dir, s)
}
fn snapshot(transform: bool) -> Snapshot {
    let s = Settings {
        transform,
        ..Settings::default()
    };
    Snapshot::capture(&s, Preset::seed())
}
fn admit(s: &Store, kind: &str, transform: bool) -> Entry {
    let id = id();
    let relative = format!("audio/{id}.wav");
    std::fs::write(s.path(&relative).unwrap(), b"original bytes").unwrap();
    let e = Entry::new(id, "microphone", "Title".into(), Some(relative), 1., 14);
    s.admit(
        e.clone(),
        Job::new(e.id.clone(), kind, snapshot(transform), kind == "initial"),
        None,
    )
    .unwrap();
    e
}
fn all(s: &Store) -> Vec<Job> {
    s.call(|db| {
        let mut q = db.prepare("SELECT document FROM jobs ORDER BY seq")?;
        let v = q
            .query_map([], |r| r.get::<_, String>(0))?
            .map(|r| Ok(serde_json::from_str(&r?)?))
            .collect();
        v
    })
    .unwrap()
}
#[test]
fn queue_capacity_reservations_and_paused_backlog() {
    let (_d, s) = store();
    for _ in 0..20 {
        admit(&s, "initial", false);
    }
    assert!(s.reserve(&id()).is_err());
    let root = s.root.clone();
    drop(s);
    let s = Store::open(root.as_path()).unwrap();
    assert_eq!(
        s.jobs()
            .unwrap()
            .iter()
            .filter(|j| j.status == "paused")
            .count(),
        20
    );
    let capture = id();
    s.reserve(&capture).unwrap();
    let e = admit(&s, "initial", false);
    let paused = s
        .jobs()
        .unwrap()
        .into_iter()
        .filter(|j| j.status == "paused")
        .map(|j| j.id)
        .collect::<Vec<_>>();
    assert!(s.resume(paused.clone(), false).is_err());
    assert_eq!(s.next().unwrap().unwrap().entry_id, e.id);
    s.release(&capture).unwrap();
    s.resume(paused[..19].to_vec(), false).unwrap();
    assert!(s.resume(paused[..1].to_vec(), false).is_err());
}
#[test]
fn resume_preserves_relative_order_and_fresh_work_precedes() {
    let (_d, s) = store();
    let a = admit(&s, "initial", false);
    let b = admit(&s, "import", false);
    let root = s.root.clone();
    drop(s);
    let s = Store::open(root.as_path()).unwrap();
    let paused = s.jobs().unwrap();
    let fresh = admit(&s, "initial", false);
    s.resume(paused.iter().rev().map(|j| j.id.clone()).collect(), false)
        .unwrap();
    let jobs = all(&s);
    assert_eq!(
        jobs.iter().map(|j| j.entry_id.clone()).collect::<Vec<_>>(),
        vec![fresh.id, a.id, b.id]
    );
    assert!(!jobs[1].eligible);
}
#[test]
fn unknown_request_needs_explicit_retry_and_is_atomic() {
    let (_d, s) = store();
    let e = admit(&s, "initial", true);
    let mut j = s.next().unwrap().unwrap();
    j.stage = "transcribing".into();
    s.checkpoint(&j).unwrap();
    let root = s.root.clone();
    drop(s);
    let s = Store::open(root.as_path()).unwrap();
    let j = s.jobs().unwrap().pop().unwrap();
    assert_eq!(j.status, "unknown");
    assert!(s.resume(vec![j.id], false).is_err());
    s.retry(&e.id, e.revision, "transcribe", snapshot(false))
        .unwrap();
    assert_eq!(
        s.jobs()
            .unwrap()
            .iter()
            .filter(|j| j.status == "queued")
            .count(),
        1
    );
    assert!(s
        .retry(&e.id, e.revision, "transcribe", snapshot(false))
        .is_err());
}
#[test]
fn retention_keeps_text_pins_and_source() {
    let (_d, s) = store();
    let e = admit(&s, "initial", false);
    let original = s.path(e.audio.as_ref().unwrap()).unwrap();
    let source = s.root.join("source.wav");
    std::fs::write(&source, b"user source").unwrap();
    let newer = admit(&s, "import", false);
    let mut settings = s.settings().unwrap();
    settings.keep_audio = Some(1);
    s.save_settings(settings).unwrap();
    s.retention().unwrap();
    assert!(original.exists());
    let mut j = s.next().unwrap().unwrap();
    j.stage = "ready".into();
    s.transcript(&j, "keep text".into(), None).unwrap();
    j.status = "complete".into();
    j.stage = "complete".into();
    s.checkpoint(&j).unwrap();
    s.retention().unwrap();
    let e = s.entry(&e.id).unwrap();
    assert!(e.audio.is_none());
    assert_eq!(e.transcript.as_deref(), Some("keep text"));
    assert!(!original.exists());
    assert_eq!(std::fs::read(source).unwrap(), b"user source");
    assert!(s.delete(&newer.id, newer.revision, true).is_err());
    assert!(s.delete(&e.id, e.revision, false).is_err());
    s.delete(&e.id, e.revision, true).unwrap();
    assert!(s.entry(&e.id).is_err());
}
#[test]
fn snapshots_and_preset_contents_are_independent() {
    let (_d, s) = store();
    let e = admit(&s, "initial", true);
    let j = s.jobs().unwrap().pop().unwrap();
    let mut preset = s.preset(&Preset::seed().id).unwrap();
    preset.system = "replacement".into();
    s.save_preset(preset).unwrap();
    let mut settings = s.settings().unwrap();
    settings.copy_raw = false;
    settings.copy_transformed = false;
    settings.transform = true;
    s.save_settings(settings.clone()).unwrap();
    assert!(s.save_settings(settings).is_err());
    let saved = s.jobs().unwrap().pop().unwrap();
    assert_eq!(saved.snapshot.preset.unwrap().system, Preset::seed().system);
    assert!(j.snapshot.copy_transformed);
    assert_eq!(s.entry(&e.id).unwrap().title, "Title");
}
#[test]
fn transcript_retry_preserves_old_transform_and_manual_metadata() {
    let (_d, s) = store();
    let e = admit(&s, "initial", true);
    let mut j = s.next().unwrap().unwrap();
    j.stage = "transforming".into();
    s.transcript(&j, "original".into(), None).unwrap();
    let e = s.entry(&e.id).unwrap();
    s.edit(&e.id, e.revision, "My title".into(), "My subtitle".into())
        .unwrap();
    j.input_revision = 1;
    j.stage = "ready".into();
    s.transformed(&j, "cleaned".into(), None).unwrap();
    j.status = "complete".into();
    s.checkpoint(&j).unwrap();
    let e = s.entry(&e.id).unwrap();
    s.retry(&e.id, e.revision, "transcribe", snapshot(true))
        .unwrap();
    let mut retry = s.next().unwrap().unwrap();
    retry.stage = "ready".into();
    s.transcript(&retry, "new text".into(), None).unwrap();
    let saved = s.entry(&e.id).unwrap();
    assert_eq!(saved.transcript_revision, 2);
    assert_eq!(saved.transformed_revision, Some(1));
    assert_eq!(saved.transformed.as_deref(), Some("cleaned"));
    assert_eq!(saved.title, "My title");
    assert_eq!(saved.subtitle, "My subtitle");
    assert!(s.transformed(&j, "late".into(), None).is_err());
}
#[derive(Default)]
struct Fakes {
    events: Mutex<Vec<String>>,
    copy_fail: bool,
    transform_fail: bool,
    asynchronous: bool,
    transcription_errors: Mutex<std::collections::VecDeque<&'static str>>,
    transformation_errors: Mutex<std::collections::VecDeque<&'static str>>,
}
#[async_trait]
impl MediaProcessor for Fakes {
    async fn probe(&self, _: &Path) -> Result<AudioInfo> {
        Ok(AudioInfo {
            duration: 1.,
            bytes: 14,
        })
    }
    async fn prepare(&self, path: &Path, _: &Path, _: &Snapshot) -> Result<PreparedAudio> {
        Ok(PreparedAudio {
            path: path.into(),
            bytes: 14,
            mime: "audio/wav",
        })
    }
}
#[async_trait]
impl TranscriptionProvider for Fakes {
    async fn transcribe(
        &self,
        _: &PreparedAudio,
        j: &Job,
        remote: &(dyn Fn(String) -> Result<()> + Send + Sync),
    ) -> Result<String> {
        self.events
            .lock()
            .unwrap()
            .push(format!("transcribe:{}", j.entry_id));
        if self.asynchronous {
            if let Some(remote) = &j.remote_id {
                self.events.lock().unwrap().push(format!("poll:{remote}"));
            } else {
                remote("existing-transcription".into())?;
                self.events.lock().unwrap().push("submit".into());
            }
        }
        if let Some(category) = self.transcription_errors.lock().unwrap().pop_front() {
            return Err(AppError::new("transcribing", category, category));
        }
        Ok("raw words".into())
    }
}
#[async_trait]
impl TransformationProvider for Fakes {
    async fn transform(&self, _: &str, _: &Preset) -> Result<String> {
        self.events.lock().unwrap().push("transform".into());
        if let Some(category) = self.transformation_errors.lock().unwrap().pop_front() {
            return Err(AppError::new("transforming", category, category));
        }
        if self.transform_fail {
            Err(AppError::new("transforming", "refused", "Refused"))
        } else {
            Ok("clean words".into())
        }
    }
}
#[async_trait]
impl ClipboardWriter for Fakes {
    async fn copy(&self, text: &str) -> Result<()> {
        self.events.lock().unwrap().push(format!("copy:{text}"));
        if self.copy_fail {
            Err(AppError::input("busy"))
        } else {
            Ok(())
        }
    }
}
impl Feedback for Fakes {
    fn changed(&self) {}
    fn sound(&self, event: &str) {
        self.events.lock().unwrap().push(format!("sound:{event}"));
    }
}
fn worker(s: Store, f: Arc<Fakes>) -> Worker {
    Worker {
        store: s,
        media: f.clone(),
        transcription: f.clone(),
        transformation: f.clone(),
        clipboard: f.clone(),
        feedback: f,
    }
}
fn realtime_job(s: &Store, outcome: Option<RealtimeOutcome>, transform: bool) -> Entry {
    let e = admit(s, "initial", transform);
    let mut j = all(s).into_iter().find(|j| j.entry_id == e.id).unwrap();
    j.snapshot.transcription_mode = TranscriptionMode::Realtime;
    j.snapshot.transcription.provider = "elevenlabs".into();
    j.snapshot.transcription.model = "scribe_v2".into();
    j.realtime = outcome;
    s.checkpoint(&j).unwrap();
    e
}
#[tokio::test]
async fn realtime_uses_confirmed_result_then_existing_delivery_and_transformation() {
    let (_d, s) = store();
    let e = realtime_job(
        &s,
        Some(RealtimeOutcome {
            result: Ok("Final streamed words.".into()),
            finalize_ms: 321,
        }),
        true,
    );
    let f = Arc::new(Fakes::default());
    worker(s.clone(), f.clone()).drain().await.unwrap();
    assert_eq!(
        *f.events.lock().unwrap(),
        vec![
            "copy:Final streamed words.",
            "sound:transcribed",
            "transform",
            "copy:clean words",
            "sound:transformed"
        ]
    );
    let saved = s.entry(&e.id).unwrap();
    assert_eq!(saved.transcription_ms, Some(321));
    assert_eq!(
        saved.transcript_model.as_deref(),
        Some("elevenlabs / scribe_v2_realtime")
    );
    assert!(s.path(saved.audio.as_ref().unwrap()).unwrap().exists());
    assert!(all(&s)[0].realtime.is_none());
}
#[tokio::test]
async fn realtime_failure_or_missing_receipt_never_reuploads_or_delivers() {
    for outcome in [
        None,
        Some(RealtimeOutcome {
            result: Err(AppError::new("transcribing", "transport", "Disconnected")),
            finalize_ms: 10,
        }),
    ] {
        let (_d, s) = store();
        let e = realtime_job(&s, outcome, false);
        let f = Arc::new(Fakes::default());
        worker(s.clone(), f.clone()).drain().await.unwrap();
        assert!(f.events.lock().unwrap().is_empty());
        assert!(s.entry(&e.id).unwrap().transcript.is_none());
        assert!(s.path(e.audio.as_ref().unwrap()).unwrap().exists());
        let snapshot = all(&s)[0].snapshot.clone();
        s.retry(&e.id, e.revision, "transcribe", snapshot).unwrap();
        assert_eq!(
            all(&s).last().unwrap().snapshot.transcription_mode,
            TranscriptionMode::Batch
        );
        worker(s.clone(), f.clone()).drain().await.unwrap();
        assert_eq!(
            s.entry(&e.id).unwrap().transcript.as_deref(),
            Some("raw words")
        );
        assert!(!f
            .events
            .lock()
            .unwrap()
            .iter()
            .any(|e| e.starts_with("copy:")));
    }
}
#[tokio::test]
async fn confirmed_stream_survives_restart_without_another_paid_request_or_copy() {
    let (dir, s) = store();
    let e = realtime_job(
        &s,
        Some(RealtimeOutcome {
            result: Ok("Durable result".into()),
            finalize_ms: 200,
        }),
        true,
    );
    drop(s);
    let s = Store::open(dir.path()).unwrap();
    let jobs = s.jobs().unwrap();
    assert_eq!(jobs[0].status, "paused");
    s.resume(vec![jobs[0].id.clone()], false).unwrap();
    let f = Arc::new(Fakes::default());
    worker(s.clone(), f.clone()).drain().await.unwrap();
    assert_eq!(
        s.entry(&e.id).unwrap().transcript.as_deref(),
        Some("Durable result")
    );
    assert_eq!(
        *f.events.lock().unwrap(),
        vec!["sound:transcribed", "transform", "sound:transformed"]
    );
}
#[test]
fn imports_and_retries_capture_batch_mode_even_when_recording_mode_is_realtime() {
    let mut snap = snapshot(false);
    snap.transcription_mode = TranscriptionMode::Realtime;
    for kind in ["import", "transcribe", "transform"] {
        let j = Job::new(id(), kind, snap.clone(), false);
        assert_eq!(j.snapshot.transcription_mode, TranscriptionMode::Batch);
    }
}
#[tokio::test]
async fn realtime_admission_journal_recovers_confirmed_result_without_reupload() {
    let (dir, s) = store();
    let identifier = id();
    let relative = format!("audio/{identifier}.wav");
    std::fs::write(s.path(&relative).unwrap(), b"original").unwrap();
    let e = Entry::new(
        identifier,
        "microphone",
        "Recovered".into(),
        Some(relative),
        1.,
        8,
    );
    let mut snap = snapshot(false);
    snap.transcription_mode = TranscriptionMode::Realtime;
    snap.transcription.provider = "elevenlabs".into();
    snap.transcription.model = "scribe_v2".into();
    let mut job = Job::new(e.id.clone(), "initial", snap, true);
    job.realtime = Some(RealtimeOutcome {
        result: Ok("Already confirmed".into()),
        finalize_ms: 25,
    });
    s.stage_admission(&e, &job).unwrap();
    drop(s);
    let s = Store::open(dir.path()).unwrap();
    let jobs = s.jobs().unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].status, "paused");
    s.resume(vec![jobs[0].id.clone()], false).unwrap();
    let f = Arc::new(Fakes::default());
    worker(s.clone(), f.clone()).drain().await.unwrap();
    assert_eq!(
        s.entry(&e.id).unwrap().transcript.as_deref(),
        Some("Already confirmed")
    );
    assert_eq!(*f.events.lock().unwrap(), vec!["sound:transcribed"]);
}
#[tokio::test]
async fn fifo_delivers_each_enabled_stage_before_its_sound() {
    let (_d, s) = store();
    let a = admit(&s, "initial", true);
    let b = admit(&s, "initial", false);
    let _c = admit(&s, "import", true);
    let f = Arc::new(Fakes::default());
    worker(s.clone(), f.clone()).drain().await.unwrap();
    let log = f.events.lock().unwrap();
    assert_eq!(
        &log[..6],
        &[
            format!("transcribe:{}", a.id),
            "copy:raw words".into(),
            "sound:transcribed".into(),
            "transform".into(),
            "copy:clean words".into(),
            "sound:transformed".into()
        ]
    );
    assert_eq!(log[6], format!("transcribe:{}", b.id));
    assert_eq!(log.iter().filter(|s| s.starts_with("copy:")).count(), 3);
    assert_eq!(all(&s).iter().filter(|j| j.status == "complete").count(), 3);
}
#[tokio::test]
async fn transform_failure_keeps_raw_and_next_job_runs() {
    let (_d, s) = store();
    let a = admit(&s, "initial", true);
    let b = admit(&s, "initial", false);
    let f = Arc::new(Fakes {
        transform_fail: true,
        ..Fakes::default()
    });
    worker(s.clone(), f.clone()).drain().await.unwrap();
    let a = s.entry(&a.id).unwrap();
    assert_eq!(a.transcript.as_deref(), Some("raw words"));
    assert!(a.transformed.is_none());
    assert!(s.entry(&b.id).unwrap().transcript.is_some());
    let log = f.events.lock().unwrap();
    assert_eq!(log.iter().filter(|e| e.starts_with("copy:")).count(), 2);
    assert!(!log.contains(&"sound:transformed".into()));
}
#[tokio::test]
async fn clipboard_failure_is_separate_and_no_stage_repeats() {
    let (_d, s) = store();
    admit(&s, "initial", false);
    let f = Arc::new(Fakes {
        copy_fail: true,
        ..Fakes::default()
    });
    let w = worker(s.clone(), f.clone());
    w.drain().await.unwrap();
    w.drain().await.unwrap();
    let jobs = all(&s);
    assert_eq!(jobs[0].status, "complete");
    assert_eq!(jobs[0].delivery, "copy_failed");
    assert_eq!(
        f.events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.starts_with("transcribe"))
            .count(),
        1
    );
}
#[tokio::test]
async fn resume_ready_never_replays_clipboard() {
    let (_d, s) = store();
    let e = admit(&s, "initial", false);
    let mut j = s.next().unwrap().unwrap();
    j.stage = "ready".into();
    s.transcript(&j, "durably saved".into(), None).unwrap();
    j.delivery = "writing".into();
    s.checkpoint(&j).unwrap();
    let root = s.root.clone();
    drop(s);
    let s = Store::open(root.as_path()).unwrap();
    s.resume(vec![j.id], false).unwrap();
    let f = Arc::new(Fakes::default());
    worker(s.clone(), f.clone()).drain().await.unwrap();
    assert!(f.events.lock().unwrap().is_empty());
    assert_eq!(
        s.entry(&e.id).unwrap().transcript.as_deref(),
        Some("durably saved")
    );
}

#[test]
fn admission_journal_recovers_finalized_import_without_automatic_delivery() {
    let (_d, s) = store();
    let identifier = id();
    let relative = format!("audio/{identifier}.wav");
    let e = Entry::new(
        identifier,
        "import",
        "source.wav".into(),
        Some(relative.clone()),
        1.,
        8,
    );
    let j = Job::new(e.id.clone(), "import", snapshot(true), false);
    s.stage_admission(&e, &j).unwrap();
    std::fs::write(s.path(&relative).unwrap(), b"original").unwrap();
    let root = s.root.clone();
    drop(s);
    let recovered = Store::open(root.as_path()).unwrap();
    assert!(recovered.entry(&e.id).unwrap().audio.is_some());
    let jobs = recovered.jobs().unwrap();
    assert_eq!(jobs[0].status, "paused");
    assert!(!jobs[0].eligible);
}
#[test]
fn multiple_audio_leases_pin_until_every_reader_finishes() {
    let (_d, s) = store();
    let e = admit(&s, "initial", false);
    let mut j = s.next().unwrap().unwrap();
    j.status = "complete".into();
    s.checkpoint(&j).unwrap();
    s.pin_audio(&e.id).unwrap();
    s.pin_audio(&e.id).unwrap();
    admit(&s, "initial", false);
    let mut settings = s.settings().unwrap();
    settings.keep_audio = Some(1);
    s.save_settings(settings).unwrap();
    s.lease(&e.id, false).unwrap();
    s.retention().unwrap();
    assert!(s.entry(&e.id).unwrap().audio.is_some());
    s.lease(&e.id, false).unwrap();
    s.retention().unwrap();
    assert!(s.entry(&e.id).unwrap().audio.is_none());
}
#[test]
fn upgrading_v1_keeps_history_and_creates_consistent_backup() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("audio")).unwrap();
    std::fs::create_dir(dir.path().join("tmp")).unwrap();
    let db = rusqlite::Connection::open(dir.path().join("x-talk.sqlite3")).unwrap();
    db.execute_batch(include_str!("../migrations/001.sql"))
        .unwrap();
    db.execute(
        "INSERT INTO settings VALUES(1,?1)",
        [serde_json::to_string(&Settings::default()).unwrap()],
    )
    .unwrap();
    let p = Preset::seed();
    db.execute(
        "INSERT INTO presets VALUES(?1,?2,?3)",
        rusqlite::params![p.id, p.name, serde_json::to_string(&p).unwrap()],
    )
    .unwrap();
    drop(db);
    let s = Store::open(dir.path()).unwrap();
    assert_eq!(s.settings().unwrap().keep_audio, Some(100));
    assert!(dir.path().join("before-migration-v2.sqlite3").exists());
    let backup =
        rusqlite::Connection::open(dir.path().join("before-migration-v2.sqlite3")).unwrap();
    let version: u32 = backup
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(version, 1);
}

#[test]
fn asynchronous_transcript_id_does_not_make_interrupted_transform_resumable() {
    let (_d, s) = store();
    admit(&s, "initial", true);
    let mut j = s.next().unwrap().unwrap();
    j.stage = "transforming".into();
    j.remote_id = Some("completed-transcription-id".into());
    s.checkpoint(&j).unwrap();
    let root = s.root.clone();
    drop(s);
    let s = Store::open(root.as_path()).unwrap();
    assert_eq!(s.jobs().unwrap()[0].status, "unknown");
}

fn asynchronous_entry(s: &Store, transform: bool) -> Entry {
    let e = admit(s, "initial", transform);
    let mut j = s.next().unwrap().unwrap();
    j.snapshot.transcription.provider = "assemblyai".into();
    j.snapshot.transcription.model = "universal-3-5-pro".into();
    j.status = "queued".into();
    s.checkpoint(&j).unwrap();
    e
}
#[tokio::test]
async fn lost_transform_response_after_async_transcription_requires_stage_retry() {
    for category in ["timeout", "transport"] {
        let (_d, s) = store();
        let e = asynchronous_entry(&s, true);
        let f = Arc::new(Fakes {
            asynchronous: true,
            transformation_errors: Mutex::new([category].into()),
            ..Fakes::default()
        });
        let w = worker(s.clone(), f.clone());
        w.drain().await.unwrap();
        let j = s.jobs().unwrap().pop().unwrap();
        assert_eq!(
            (j.status.as_str(), j.stage.as_str()),
            ("unknown", "transforming")
        );
        assert!(j.error.as_ref().unwrap().message.contains("another charge"));
        assert!(s.resume(vec![j.id.clone()], false).is_err());
        // Also reject legacy paused transformation records, even with a remote transcription ID.
        let mut legacy = j.clone();
        legacy.status = "paused".into();
        s.checkpoint(&legacy).unwrap();
        assert!(s.resume(vec![legacy.id], false).is_err());
        let root = s.root.clone();
        drop(w);
        drop(s);
        let s = Store::open(root.as_path()).unwrap();
        assert_eq!(s.jobs().unwrap()[0].status, "unknown");
        assert!(s.resume(vec![j.id], false).is_err());
        let before = s.entry(&e.id).unwrap();
        s.retry(&e.id, before.revision, "transform", snapshot(true))
            .unwrap();
        worker(s.clone(), f.clone()).drain().await.unwrap();
        let after = s.entry(&e.id).unwrap();
        assert_eq!(after.transcript, before.transcript);
        assert_eq!(after.transcript_revision, before.transcript_revision);
        assert_eq!(after.transformed.as_deref(), Some("clean words"));
        let events = f.events.lock().unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|e| e.starts_with("transcribe:"))
                .count(),
            1
        );
        assert_eq!(
            events.iter().filter(|e| e.as_str() == "transform").count(),
            2
        );
        assert_eq!(
            events
                .iter()
                .filter(|e| e.starts_with("copy:"))
                .collect::<Vec<_>>(),
            vec!["copy:raw words"]
        );
    }
}
#[tokio::test]
async fn async_resume_polls_existing_id_and_replaces_or_clears_attempt_errors() {
    let (_d, s) = store();
    asynchronous_entry(&s, false);
    let f = Arc::new(Fakes {
        asynchronous: true,
        transcription_errors: Mutex::new(["poll_timeout", "transport"].into()),
        ..Fakes::default()
    });
    let w = worker(s.clone(), f.clone());
    for expected in ["poll_timeout", "transport"] {
        w.drain().await.unwrap();
        let j = s.jobs().unwrap().pop().unwrap();
        assert_eq!(j.status, "paused");
        assert_eq!(j.error.unwrap().category, expected);
        s.resume(vec![j.id], false).unwrap();
        assert!(s.jobs().unwrap()[0].error.is_none());
    }
    w.drain().await.unwrap();
    let j = s.jobs().unwrap().pop().unwrap();
    assert_eq!(j.status, "complete");
    assert!(j.error.is_none());
    assert_eq!(j.delivery, "manual_only");
    assert!(JobView::from(j).error.is_none());
    let events = f.events.lock().unwrap();
    assert_eq!(events.iter().filter(|e| e.as_str() == "submit").count(), 1);
    assert_eq!(
        events
            .iter()
            .filter(|e| e.as_str() == "poll:existing-transcription")
            .count(),
        2
    );
    assert!(!events.iter().any(|e| e.starts_with("copy:")));
}
#[test]
fn startup_clears_stale_error_from_legacy_completed_jobs() {
    let (_d, s) = store();
    admit(&s, "initial", false);
    let mut j = s.next().unwrap().unwrap();
    j.status = "complete".into();
    j.stage = "complete".into();
    j.error = Some(AppError::new("transcribing", "poll_timeout", "old error"));
    s.checkpoint(&j).unwrap();
    let root = s.root.clone();
    drop(s);
    let s = Store::open(root.as_path()).unwrap();
    assert!(s.jobs().unwrap()[0].error.is_none());
}

#[test]
fn import_recovery_isolated_at_each_journal_and_admission_boundary() {
    for point in [
        "created",
        "partial",
        "synced",
        "published",
        "renamed",
        "admitted",
    ] {
        let (_d, s) = store();
        let unrelated = admit(&s, "initial", false);
        let identifier = id();
        let relative = format!("audio/{identifier}.wav");
        let e = Entry::new(
            identifier.clone(),
            "import",
            "source.wav".into(),
            Some(relative.clone()),
            1.,
            8,
        );
        let j = Job::new(e.id.clone(), "import", snapshot(true), false);
        let audio = s.path(&relative).unwrap();
        let part = s
            .root
            .join("audio")
            .join(format!("{identifier}.import.part"));
        let journal = s.root.join("audio").join(format!("{identifier}.admission"));
        let temporary = journal.with_extension("admission.new");
        std::fs::write(&part, b"original").unwrap();
        match point {
            "created" => std::fs::write(&temporary, b"").unwrap(),
            "partial" => std::fs::write(&journal, b"[{\"id\":").unwrap(),
            "synced" => {
                std::fs::write(&temporary, serde_json::to_vec(&(&e, &j)).unwrap()).unwrap();
                std::fs::OpenOptions::new()
                    .write(true)
                    .open(&temporary)
                    .unwrap()
                    .sync_all()
                    .unwrap();
            }
            _ => {
                s.stage_admission(&e, &j).unwrap();
            }
        }
        if ["renamed", "admitted"].contains(&point) {
            std::fs::rename(&part, &audio).unwrap();
        }
        if point == "admitted" {
            s.admit(e.clone(), j, None).unwrap();
        }
        let root = s.root.clone();
        drop(s);
        let s = Store::open(root.as_path()).unwrap();
        assert_eq!(s.entry(&unrelated.id).unwrap().title, "Title");
        if ["created", "partial"].contains(&point) {
            assert!(s.entry(&identifier).is_err());
            assert_eq!(std::fs::read(&part).unwrap(), b"original");
            assert!(!s.recovery_warnings().is_empty());
            assert!(std::fs::read_dir(s.root.join("audio")).unwrap().any(|p| p
                .unwrap()
                .path()
                .extension()
                .is_some_and(|s| s == "invalid")));
        } else {
            assert_eq!(std::fs::read(&audio).unwrap(), b"original");
            assert!(s.entry(&identifier).unwrap().audio.is_some());
            let jobs = s.jobs().unwrap();
            let recovered = jobs.iter().find(|j| j.entry_id == identifier).unwrap();
            assert_eq!(recovered.status, "paused");
            assert!(!recovered.eligible);
            assert_eq!(jobs.iter().filter(|j| j.entry_id == identifier).count(), 1);
            assert!(s.next().unwrap().is_none());
        }
        let fresh = admit(&s, "initial", false);
        assert!(s.entry(&fresh.id).is_ok());
        assert_eq!(s.next().unwrap().unwrap().entry_id, fresh.id);
        // Repeating recovery neither duplicates an admission nor blocks future startup.
        let root = s.root.clone();
        drop(s);
        let s = Store::open(root.as_path()).unwrap();
        assert!(s.entry(&unrelated.id).is_ok());
    }
}

#[test]
fn recordings_search_sort_and_paging_cover_the_full_library_and_literal_queries() {
    let (_dir, s) = store();
    for i in 0..65 {
        let mut entry = Entry::new(
            id(),
            "import",
            format!("Recording {i:02}"),
            None,
            i as f64,
            (65 - i) * 100,
        );
        entry.subtitle = format!("Subtitle {i:02}");
        entry.transcript = Some(format!("raw words {i:02}"));
        entry.transformed = Some(format!("refined words {i:02}"));
        if i == 0 {
            entry.transcript = Some("literal 100%_ and 'quoted' value".into());
        }
        s.interrupted_entry(entry).unwrap();
    }
    let query = |search: &str, sort: &str, descending, offset| RecordingQuery {
        search: search.into(),
        sort: sort.into(),
        descending,
        offset,
        page_size: 50,
    };
    let first = s.recordings(query("", "title", false, 0)).unwrap();
    assert_eq!(first.total, 65);
    assert_eq!(first.entries.len(), 50);
    assert_eq!(first.entries[0].title, "Recording 00");
    let mut small = query("", "title", false, 0);
    small.page_size = 25;
    assert_eq!(s.recordings(small).unwrap().entries.len(), 25);
    let mut large = query("", "title", false, 0);
    large.page_size = 100;
    assert_eq!(s.recordings(large).unwrap().entries.len(), 65);
    let mut invalid = query("", "title", false, 0);
    invalid.page_size = 1;
    assert!(s.recordings(invalid).is_err());
    assert_eq!(
        s.recordings(query("Recording 64", "created", true, 0))
            .unwrap()
            .total,
        0
    );
    assert_eq!(
        s.recordings(query("raw words 64", "created", true, 0))
            .unwrap()
            .entries[0]
            .title,
        "Recording 64"
    );

    let last = s.recordings(query("", "title", false, 50)).unwrap();
    assert_eq!(last.entries.len(), 15);
    assert_eq!(last.entries[14].title, "Recording 64");
    assert_eq!(
        s.recordings(query("REFINED WORDS 34", "created", true, 0))
            .unwrap()
            .entries[0]
            .title,
        "Recording 34"
    );
    assert_eq!(
        s.recordings(query("Subtitle 05", "created", true, 0))
            .unwrap()
            .total,
        0
    );
    assert_eq!(
        s.recordings(query("100%_ and 'quoted'", "created", true, 0))
            .unwrap()
            .total,
        1
    );
    assert_eq!(
        s.recordings(query("", "bytes", false, 0)).unwrap().entries[0].title,
        "Recording 64"
    );
    assert_eq!(
        s.recordings(query("", "duration", true, 0))
            .unwrap()
            .entries[0]
            .duration,
        64.
    );
    assert!(s
        .recordings(query("", "title'); DROP TABLE entries; --", true, 0))
        .is_err());
    assert_eq!(
        s.recordings(query("not present", "title", false, 0))
            .unwrap()
            .total,
        0
    );
}
#[test]
fn manual_transcript_edits_mark_output_stale_and_reject_pending_or_stale_writes() {
    let (_dir, s) = store();
    let mut e = Entry::new(id(), "import", "Title".into(), None, 1., 0);
    e.transcript = Some("Before".into());
    e.transcript_revision = 1;
    e.transformed = Some("Previous refined output".into());
    e.transformed_revision = Some(1);
    s.interrupted_entry(e.clone()).unwrap();
    s.edit_recording(
        &e.id,
        1,
        "Edited".into(),
        "User subtitle".into(),
        "Corrected".into(),
    )
    .unwrap();
    let changed = s.entry(&e.id).unwrap();
    assert_eq!(changed.transcript_revision, 2);
    assert_eq!(changed.transformed_revision, Some(1));
    assert_eq!(changed.transformed, e.transformed);
    assert!(changed.subtitle_manual);
    assert!(s
        .edit_recording(&e.id, 1, "Stale".into(), "".into(), "Lost change".into())
        .is_err());
    s.edit_recording(
        &e.id,
        changed.revision,
        "Only title".into(),
        "".into(),
        "Corrected".into(),
    )
    .unwrap();
    assert_eq!(s.entry(&e.id).unwrap().transcript_revision, 2);
    let pending = admit(&s, "initial", false);
    assert!(s
        .edit_recording(
            &pending.id,
            pending.revision,
            "Title".into(),
            "".into(),
            "Should wait".into()
        )
        .is_err());
    assert!(s.entry(&pending.id).unwrap().transcript.is_none());
}

#[tokio::test]
async fn stage_times_follow_saved_results_and_survive_restart_and_failed_retries() {
    struct Timed;
    #[async_trait]
    impl TranscriptionProvider for Timed {
        async fn transcribe(
            &self,
            _: &PreparedAudio,
            _: &Job,
            _: &(dyn Fn(String) -> Result<()> + Send + Sync),
        ) -> Result<String> {
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            Ok("Timed transcript".into())
        }
    }
    #[async_trait]
    impl TransformationProvider for Timed {
        async fn transform(&self, _: &str, _: &Preset) -> Result<String> {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            Ok("Timed transformation".into())
        }
    }
    let (dir, s) = store();
    let e = admit(&s, "initial", true);
    let f = Arc::new(Fakes::default());
    let mut w = worker(s.clone(), f);
    w.transcription = Arc::new(Timed);
    w.transformation = Arc::new(Timed);
    w.drain().await.unwrap();
    let saved = s.entry(&e.id).unwrap();
    assert!(saved.transcription_ms.unwrap() >= 20);
    assert!(saved.transformation_ms.unwrap() >= 45);
    drop(w);
    drop(s);
    let s = Store::open(dir.path()).unwrap();
    let loaded = s.entry(&e.id).unwrap();
    assert_eq!(saved.transcription_ms, loaded.transcription_ms);
    assert_eq!(saved.transformation_ms, loaded.transformation_ms);
    s.retry(&loaded.id, loaded.revision, "transform", snapshot(true))
        .unwrap();
    worker(
        s.clone(),
        Arc::new(Fakes {
            transform_fail: true,
            ..Fakes::default()
        }),
    )
    .drain()
    .await
    .unwrap();
    let failed = s.entry(&e.id).unwrap();
    assert_eq!(failed.transformed, loaded.transformed);
    assert_eq!(failed.transformation_ms, loaded.transformation_ms);
    s.edit_recording(
        &failed.id,
        failed.revision,
        failed.title.clone(),
        failed.subtitle.clone(),
        "Manually corrected text".into(),
    )
    .unwrap();
    let edited = s.entry(&e.id).unwrap();
    assert!(edited.transcription_ms.is_none());
    assert_eq!(edited.transformation_ms, loaded.transformation_ms);
    let mut legacy = serde_json::to_value(&edited).unwrap();
    legacy.as_object_mut().unwrap().remove("transcription_ms");
    legacy.as_object_mut().unwrap().remove("transformation_ms");
    let legacy: Entry = serde_json::from_value(legacy).unwrap();
    assert!(legacy.transcription_ms.is_none());
    assert!(legacy.transformation_ms.is_none());
}

#[test]
fn single_transformation_uses_the_previous_active_preset_and_preserves_archives_and_jobs() {
    let (dir, s) = store();
    let archived = s.preset(&Preset::seed().id).unwrap();
    let mut custom = Preset::seed();
    custom.id = id();
    custom.name = "My existing cleanup".into();
    custom.description = "Keep this legacy metadata".into();
    custom.system = "My carefully edited prompt".into();
    custom.reasoning = Some("low".into());
    let custom = s.save_preset(custom).unwrap();
    let mut settings = s.settings().unwrap();
    settings.preset_id = custom.id.clone();
    settings.transform = true;
    let settings = s.save_settings(settings).unwrap();
    let original = serde_json::to_value(&custom).unwrap();
    let e = Entry::new(id(), "import", "Saved work".into(), None, 1., 0);
    s.admit(
        e.clone(),
        Job::new(
            e.id.clone(),
            "initial",
            Snapshot::capture(&settings, custom.clone()),
            false,
        ),
        None,
    )
    .unwrap();
    let config = s.transformation().unwrap();
    assert_eq!(config.system, custom.system);
    assert_eq!(config.reasoning, custom.reasoning);
    let exposed = serde_json::to_value(&config).unwrap();
    assert!(exposed.get("id").is_none());
    assert!(exposed.get("name").is_none());
    assert!(exposed.get("description").is_none());
    let mut edit = config;
    edit.system = "Updated single transformation".into();
    edit.model = "gpt-5-mini".into();
    edit.reasoning = Some("minimal".into());
    let updated = s.save_transformation(edit).unwrap();
    assert_eq!(updated.revision, custom.revision + 1);
    assert_eq!(s.presets().unwrap().len(), 2);
    assert_eq!(
        serde_json::to_value(s.preset(&archived.id).unwrap()).unwrap(),
        serde_json::to_value(archived).unwrap()
    );
    let kept = s.preset(&custom.id).unwrap();
    assert_eq!(kept.id, custom.id);
    assert_eq!(kept.name, custom.name);
    assert_eq!(kept.description, custom.description);
    let pending = s
        .jobs()
        .unwrap()
        .into_iter()
        .find(|j| j.entry_id == e.id)
        .unwrap();
    assert_eq!(
        serde_json::to_value(pending.snapshot.preset.unwrap()).unwrap(),
        original
    );
    assert_eq!(s.snapshot().unwrap().preset.unwrap().system, updated.system);
    drop(s);
    let s = Store::open(dir.path()).unwrap();
    assert_eq!(s.settings().unwrap().preset_id, custom.id);
    assert_eq!(s.transformation().unwrap().system, updated.system);
    assert_eq!(s.presets().unwrap().len(), 2);
}
#[test]
fn single_transformation_rejects_stale_and_invalid_edits_without_partial_writes() {
    let (_dir, s) = store();
    let original = s.transformation().unwrap();
    let mut edit = original.clone();
    edit.system = "Changed".into();
    let saved = s.save_transformation(edit).unwrap();
    assert!(s.save_transformation(original).is_err());
    let mut invalid = saved.clone();
    invalid.user = "No input placeholder".into();
    assert!(s.save_transformation(invalid).is_err());
    assert_eq!(
        serde_json::to_value(s.transformation().unwrap()).unwrap(),
        serde_json::to_value(saved).unwrap()
    );
    assert_eq!(s.presets().unwrap().len(), 1);
}

#[test]
fn clear_recordings_requires_confirmation_and_clears_all_pages_but_preserves_settings_and_sources()
{
    let (dir, s) = store();
    let source = dir.path().join("imported-source.wav");
    std::fs::write(&source, b"external source").unwrap();
    let original_settings = serde_json::to_value(s.settings().unwrap()).unwrap();
    let original_transformation = serde_json::to_value(s.transformation().unwrap()).unwrap();
    let mut managed = None;
    for i in 0..65 {
        let mut e = Entry::new(id(), "import", format!("Recording {i}"), None, 1., 0);
        e.transcript = Some(format!("Raw {i}"));
        e.transformed = Some(format!("Transformed {i}"));
        if i == 0 {
            let relative = format!("audio/{}.wav", e.id);
            let path = s.path(&relative).unwrap();
            std::fs::write(&path, b"managed copy").unwrap();
            e.audio = Some(relative);
            managed = Some(path);
        }
        s.interrupted_entry(e).unwrap();
    }
    let paused = admit(&s, "initial", false);
    let paused_audio = s.path(paused.audio.as_ref().unwrap()).unwrap();
    let mut j = s.jobs().unwrap().pop().unwrap();
    j.status = "paused".into();
    s.checkpoint(&j).unwrap();
    let query = || RecordingQuery {
        search: String::new(),
        sort: "created".into(),
        descending: true,
        offset: 0,
        page_size: 50,
    };
    assert!(s.clear_recordings(false).is_err());
    assert_eq!(s.recordings(query()).unwrap().total, 66);
    assert!(managed.as_ref().unwrap().exists());
    assert_eq!(s.clear_recordings(true).unwrap(), 66);
    assert_eq!(s.recordings(query()).unwrap().total, 0);
    assert!(s.jobs().unwrap().is_empty());
    assert!(!managed.unwrap().exists());
    assert!(!paused_audio.exists());
    assert_eq!(std::fs::read(&source).unwrap(), b"external source");
    assert_eq!(
        serde_json::to_value(s.settings().unwrap()).unwrap(),
        original_settings
    );
    assert_eq!(
        serde_json::to_value(s.transformation().unwrap()).unwrap(),
        original_transformation
    );
    drop(s);
    let s = Store::open(dir.path()).unwrap();
    assert!(s.entries(0).unwrap().is_empty());
    assert_eq!(s.clear_recordings(true).unwrap(), 0);
}
#[test]
fn clear_recordings_refuses_busy_jobs_leases_and_reservations_without_partial_deletion() {
    let (_dir, s) = store();
    let e = admit(&s, "initial", false);
    let path = s.path(e.audio.as_ref().unwrap()).unwrap();
    assert!(s.clear_recordings(true).is_err());
    assert!(s.entry(&e.id).is_ok());
    assert!(path.exists());
    let mut job = s.jobs().unwrap().pop().unwrap();
    job.status = "running".into();
    s.checkpoint(&job).unwrap();
    assert!(s.clear_recordings(true).is_err());
    job.status = "paused".into();
    s.checkpoint(&job).unwrap();
    s.pin_audio(&e.id).unwrap();
    assert!(s.clear_recordings(true).is_err());
    s.lease(&e.id, false).unwrap();
    let reservation = id();
    s.reserve(&reservation).unwrap();
    assert!(s.clear_recordings(true).is_err());
    assert!(s.entry(&e.id).is_ok());
    assert!(path.exists());
    s.release(&reservation).unwrap();
    assert_eq!(s.clear_recordings(true).unwrap(), 1);
}
#[test]
fn clear_recordings_can_retry_unfinished_managed_file_cleanup() {
    let (_dir, s) = store();
    let identifier = id();
    let relative = format!("audio/{identifier}.wav");
    let path = s.path(&relative).unwrap();
    // A directory in place of a managed audio file makes unlink fail on both supported test hosts.
    std::fs::create_dir(&path).unwrap();
    let e = Entry::new(
        identifier,
        "import",
        "Blocked audio".into(),
        Some(relative),
        1.,
        0,
    );
    s.interrupted_entry(e).unwrap();
    assert!(s.clear_recordings(true).is_err());
    assert!(path.is_dir());
    std::fs::remove_dir(path).unwrap();
    assert_eq!(s.clear_recordings(true).unwrap(), 0);
    let remaining: i64 = s
        .call(|db| Ok(db.query_row("SELECT count(*) FROM entries", [], |r| r.get(0))?))
        .unwrap();
    assert_eq!(remaining, 0);
}

#[test]
fn latest_entry_follows_library_changes_without_loading_a_history_page() {
    let (_dir, s) = store();
    assert!(s.latest_entry().unwrap().is_none());
    let first = admit(&s, "initial", false);
    assert_eq!(s.latest_entry().unwrap().unwrap().id, first.id);
    let second = Entry::new(id(), "import", "Newest".into(), None, 1., 0);
    s.interrupted_entry(second.clone()).unwrap();
    assert_eq!(s.latest_entry().unwrap().unwrap().id, second.id);
    s.delete(&second.id, second.revision, true).unwrap();
    assert_eq!(s.latest_entry().unwrap().unwrap().id, first.id);
}

#[test]
fn updates_wait_for_active_work_but_allow_preserved_paused_history() {
    let (_dir, s) = store();
    assert!(!s.update_busy().unwrap());
    let e = admit(&s, "initial", false);
    assert!(s.update_busy().unwrap());
    let mut job = s.jobs().unwrap().pop().unwrap();
    job.status = "running".into();
    s.checkpoint(&job).unwrap();
    assert!(s.update_busy().unwrap());
    job.status = "paused".into();
    s.checkpoint(&job).unwrap();
    assert!(!s.update_busy().unwrap());
    s.pin_audio(&e.id).unwrap();
    assert!(s.update_busy().unwrap());
    s.lease(&e.id, false).unwrap();
    let reservation = id();
    s.reserve(&reservation).unwrap();
    assert!(s.update_busy().unwrap());
    s.release(&reservation).unwrap();
    assert!(!s.update_busy().unwrap());
    assert!(s.entry(&e.id).is_ok());
}

#[test]
fn updates_default_on_for_older_settings_and_opt_out_survives_restart() {
    let mut json = serde_json::to_value(Settings::default()).unwrap();
    json.as_object_mut().unwrap().remove("check_updates");
    assert!(
        serde_json::from_value::<Settings>(json)
            .unwrap()
            .check_updates
    );
    let (dir, s) = store();
    let mut settings = s.settings().unwrap();
    settings.check_updates = false;
    s.save_settings(settings).unwrap();
    drop(s);
    assert!(
        !Store::open(dir.path())
            .unwrap()
            .settings()
            .unwrap()
            .check_updates
    );
}

#[tokio::test]
async fn staged_copy_respects_both_toggles_and_eligibility() {
    for transform in [false, true] {
        for raw in [false, true] {
            for cleaned in [false, true] {
                for eligible in [false, true] {
                    let (_d, s) = store();
                    admit(&s, "initial", transform);
                    let mut j = all(&s).pop().unwrap();
                    j.snapshot.copy_raw = raw;
                    j.snapshot.copy_transformed = cleaned;
                    j.eligible = eligible;
                    s.checkpoint(&j).unwrap();
                    let f = Arc::new(Fakes::default());
                    worker(s.clone(), f.clone()).drain().await.unwrap();
                    let events = f.events.lock().unwrap();
                    let copies = events
                        .iter()
                        .filter(|e| e.starts_with("copy:"))
                        .map(String::as_str)
                        .collect::<Vec<_>>();
                    let mut expected = vec![];
                    if eligible && raw {
                        expected.push("copy:raw words");
                    }
                    if eligible && transform && cleaned {
                        expected.push("copy:clean words");
                    }
                    assert_eq!(copies, expected);
                    assert_eq!(
                        all(&s)[0].delivery,
                        if expected.is_empty() {
                            "manual_only"
                        } else {
                            "copied"
                        }
                    );
                }
            }
        }
    }
}

#[tokio::test]
async fn failed_intermediate_copy_does_not_prevent_transformation_or_repeat_providers() {
    let (_d, s) = store();
    admit(&s, "initial", true);
    let f = Arc::new(Fakes {
        copy_fail: true,
        ..Fakes::default()
    });
    let w = worker(s.clone(), f.clone());
    w.drain().await.unwrap();
    w.drain().await.unwrap();
    let events = f.events.lock().unwrap();
    assert_eq!(events.iter().filter(|e| e.starts_with("copy:")).count(), 2);
    assert_eq!(
        events.iter().filter(|e| e.as_str() == "transform").count(),
        1
    );
    assert_eq!(all(&s)[0].status, "complete");
    assert_eq!(all(&s)[0].delivery, "copy_failed");
}

struct WaitingProvider {
    entered: tokio::sync::Notify,
    wait_in_transform: bool,
}
#[async_trait]
impl TranscriptionProvider for WaitingProvider {
    async fn transcribe(
        &self,
        _: &PreparedAudio,
        _: &Job,
        _: &(dyn Fn(String) -> Result<()> + Send + Sync),
    ) -> Result<String> {
        if !self.wait_in_transform {
            self.entered.notify_one();
            std::future::pending::<()>().await;
        }
        Ok("saved transcript".into())
    }
}
#[async_trait]
impl TransformationProvider for WaitingProvider {
    async fn transform(&self, _: &str, _: &Preset) -> Result<String> {
        self.entered.notify_one();
        std::future::pending::<()>().await;
        unreachable!()
    }
}
#[tokio::test]
async fn cancellation_stops_each_provider_stage_and_preserves_completed_work() {
    for wait_in_transform in [false, true] {
        let (_dir, store) = store();
        let entry = admit(&store, "import", true);
        let fakes = Arc::new(Fakes::default());
        let provider = Arc::new(WaitingProvider {
            entered: tokio::sync::Notify::new(),
            wait_in_transform,
        });
        let worker = Worker {
            store: store.clone(),
            media: fakes.clone(),
            transcription: provider.clone(),
            transformation: provider.clone(),
            clipboard: fakes.clone(),
            feedback: fakes.clone(),
        };
        let (result, ()) = tokio::join!(worker.run_next(), async {
            provider.entered.notified().await;
            assert!(store.ensure_processing_idle().is_err());
            store.cancel_processing().unwrap();
            assert!(
                store.processing_busy().unwrap(),
                "must remain busy until worker acknowledges cancellation"
            );
        });
        assert!(result.unwrap());
        assert!(!store.processing_busy().unwrap());
        let saved = store.entry(&entry.id).unwrap();
        assert_eq!(
            saved.transcript.as_deref(),
            if wait_in_transform {
                Some("saved transcript")
            } else {
                None
            }
        );
        assert!(saved.transformed.is_none());
        assert!(store.path(saved.audio.as_ref().unwrap()).unwrap().exists());
        assert_eq!(all(&store)[0].status, "canceled");
        assert!(!fakes
            .events
            .lock()
            .unwrap()
            .iter()
            .any(|e| e.starts_with("copy:")));
        let mut settings = store.settings().unwrap();
        settings.keep_audio = Some(1);
        store.save_settings(settings).unwrap();
        let _newer = admit(&store, "import", false);
        let mut newest = store.next().unwrap().unwrap();
        newest.status = "complete".into();
        newest.stage = "complete".into();
        store.checkpoint(&newest).unwrap();
        store.retention().unwrap();
        assert!(
            store.entry(&entry.id).unwrap().audio.is_some(),
            "cancellation must preserve original audio even beyond the retention count"
        );
        store.ensure_processing_idle().unwrap();
        let revision = store.entry(&entry.id).unwrap().revision;
        store
            .retry(
                &entry.id,
                revision,
                if wait_in_transform {
                    "transform"
                } else {
                    "transcribe"
                },
                snapshot(true),
            )
            .unwrap();
        assert!(store.processing_busy().unwrap());
    }
}
#[test]
fn cancellation_rejects_late_writes_and_survives_restart_without_resubmission() {
    let (_dir, store) = store();
    let entry = admit(&store, "import", true);
    let job = store.next().unwrap().unwrap();
    store.cancel_processing().unwrap();
    assert_eq!(store.checkpoint(&job).unwrap_err().category, "canceled");
    assert_eq!(
        store
            .transcript(&job, "late result".into(), Some(12))
            .unwrap_err()
            .category,
        "canceled"
    );
    assert!(store.entry(&entry.id).unwrap().transcript.is_none());
    let root = store.root.clone();
    drop(store);
    let store = Store::open(root.as_path()).unwrap();
    assert_eq!(all(&store)[0].status, "canceled");
    assert!(store.next().unwrap().is_none());
    assert!(!store.processing_busy().unwrap());
}
#[test]
fn serial_admission_accounts_for_reservations_jobs_and_cancellation() {
    let (_dir, store) = store();
    store.ensure_processing_idle().unwrap();
    store.reserve("import reservation").unwrap();
    assert!(store.ensure_processing_idle().is_err());
    store.release("import reservation").unwrap();
    let _ = admit(&store, "import", false);
    assert!(store.ensure_processing_idle().is_err());
    store.cancel_processing().unwrap();
    store.ensure_processing_idle().unwrap();
    assert!(store.next().unwrap().is_none());
}
