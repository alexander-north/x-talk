use std::collections::BTreeMap;
use x_talk::meetings::{domain::*, export, store::MeetingStore};

fn fixture() -> Meeting {
    let mut m = Meeting::new(
        "A \"meeting\"\nwith evidence".into(),
        SourceKind::Audio,
        Some("call.wav".into()),
    );
    m.transcript = Some(Transcript {
        generation: "generation-1".into(),
        provider: "assemblyai".into(),
        model: Some("test".into()),
        turns: vec![
            Turn {
                id: "a".into(),
                speaker: "A".into(),
                start: 0.0,
                end: 10.0,
                text: "Original words".into(),
                words: vec![Word {
                    text: "Original".into(),
                    start: Some(0.0),
                    end: Some(1.0),
                    confidence: Some(0.9),
                }],
            },
            Turn {
                id: "b".into(),
                speaker: "A".into(),
                start: 5.0,
                end: 15.0,
                text: "More words".into(),
                words: vec![],
            },
            Turn {
                id: "c".into(),
                speaker: "B".into(),
                start: 8.0,
                end: 20.0,
                text: "Other speaker".into(),
                words: vec![],
            },
            Turn {
                id: "d".into(),
                speaker: "C".into(),
                start: 20.0,
                end: 25.0,
                text: "Final speaker".into(),
                words: vec![],
            },
        ],
        names: BTreeMap::new(),
        corrections: BTreeMap::new(),
        merges: vec![],
    });
    m.attempt.stage = Stage::Complete;
    m
}

#[test]
fn json_preserves_originals_and_union_coverage_after_corrections_and_chained_merge() {
    let mut m = fixture();
    m.correct(
        "a",
        Some(Correction {
            text: Some("Edited \"text\"".into()),
            speaker: None,
        }),
    )
    .unwrap();
    m.merge("A", "B").unwrap();
    m.merge("B", "C").unwrap();
    m.name_speaker("C", "Alex").unwrap();
    let v: serde_json::Value = serde_json::from_str(&export::json(&m).unwrap()).unwrap();
    assert_eq!(v["format"], "x-talk.meeting");
    let t = &v["transcript"];
    assert_eq!(t["speakers"].as_array().unwrap().len(), 1);
    assert_eq!(t["speakers"][0]["estimated_turn_coverage_seconds"], 25.0);
    assert_eq!(t["speakers"][0]["word_count"], 8);
    assert_eq!(t["turns"][0]["text"], "Edited \"text\"");
    assert_eq!(t["turns"][0]["original_text"], "Original words");
    assert_eq!(t["turns"][0]["original_words"][0]["text"], "Original");
    assert_eq!(t["turns"][0]["speaker_id"], "C");
    m.undo_merge().unwrap();
    assert_eq!(
        export::project(&m)
            .unwrap()
            .transcript
            .unwrap()
            .speakers
            .len(),
        2
    );
    m.undo_merge().unwrap();
    m.correct("a", None).unwrap();
    assert_eq!(
        export::project(&m).unwrap().transcript.unwrap().turns[0].text,
        "Original words"
    );
}

#[test]
fn brief_staleness_tracks_content_and_generation_not_metadata() {
    let mut m = fixture();
    m.brief.content = Some(BriefContent {
        title: "Title".into(),
        summary: "Summary".into(),
        topics: vec![],
        decisions: vec![],
        actions: vec![],
        open_questions: vec![],
        suggested_follow_ups: vec![],
    });
    m.brief.input_generation = Some("generation-1".into());
    m.brief.input_revision = Some(0);
    m.title = "Manual title".into();
    m.revision += 1;
    assert!(!m.brief_stale());
    m.name_speaker("A", "Alex").unwrap();
    assert!(m.brief_stale());
}

#[test]
fn store_guards_revision_attempt_deletion_and_resumes_only_known_submissions() {
    let dir = tempfile::tempdir().unwrap();
    let store = MeetingStore::open(dir.path()).unwrap();
    let mut m = fixture();
    m.attempt.stage = Stage::Submitting;
    store.insert(m.clone()).unwrap();
    store
        .edit(&m.id, 0, |m| {
            m.title = "Manual".into();
            m.manual_title = true;
            Ok(())
        })
        .unwrap();
    assert!(store.edit(&m.id, 0, |_| Ok(())).is_err());
    assert!(store.publish(&m.id, "wrong-attempt", |_| Ok(())).is_err());
    drop(store);
    let store = MeetingStore::open(dir.path()).unwrap();
    let current = store.get(&m.id).unwrap();
    assert_eq!(current.attempt.stage, Stage::SubmissionUnknown);
    assert_eq!(current.title, "Manual");
    store.delete(&m.id, current.revision).unwrap();
    assert!(store.publish(&m.id, &m.attempt.id, |_| Ok(())).is_err());
}

#[test]
fn retention_counts_meetings_protects_leases_and_keeps_text() {
    let dir = tempfile::tempdir().unwrap();
    let store = MeetingStore::open(dir.path()).unwrap();
    let mut ids = Vec::new();
    for index in 0..3 {
        let mut m = fixture();
        m.added_at = format!("2026-01-0{}", index + 1);
        let path = store.audio_path(&m.id, "original.wav").unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"audio").unwrap();
        let mix = store.audio_path(&m.id, "mix.wav").unwrap();
        std::fs::write(mix, b"mix").unwrap();
        m.audio = Some(Audio {
            download: "original.wav".into(),
            prepared: None,
            assets: vec!["original.wav".into(), "mix.wav".into()],
        });
        ids.push(m.id.clone());
        store.insert(m).unwrap();
    }
    let lease = store.lease(&ids[0]).unwrap();
    store.set_retention(Some(1)).unwrap();
    assert!(store.get(&ids[0]).unwrap().audio.is_some());
    assert!(store.get(&ids[1]).unwrap().audio.is_none());
    drop(lease); // Releasing the last protection reapplies retention automatically.
    let oldest = store.get(&ids[0]).unwrap();
    assert!(oldest.audio.is_none());
    assert!(oldest.transcript.is_some());
    assert!(!store.audio_path(&ids[0], "mix.wav").unwrap().exists());
    assert!(!export::project(&oldest).unwrap().audio_available);
    assert!(store.get(&ids[2]).unwrap().audio.is_some());
    assert!(store.audio_path(&ids[2], "../escape").is_err());
}

#[test]
fn rejects_invalid_timestamps_cycles_and_missing_brief_evidence() {
    let mut m = fixture();
    m.transcript.as_mut().unwrap().turns[0].end = f64::NAN;
    assert!(export::json(&m).is_err());
    let mut m = fixture();
    m.transcript.as_mut().unwrap().merges = vec![
        SpeakerMerge {
            from: "A".into(),
            into: "B".into(),
        },
        SpeakerMerge {
            from: "B".into(),
            into: "A".into(),
        },
    ];
    assert!(export::json(&m).is_err());
    let brief = BriefContent {
        title: "Title".into(),
        summary: "Summary".into(),
        topics: vec![Evidence {
            text: "Claim".into(),
            segment_ids: vec!["nonexistent".into()],
        }],
        decisions: vec![],
        actions: vec![],
        open_questions: vec![],
        suggested_follow_ups: vec![],
    };
    assert!(brief
        .validate(fixture().transcript.as_ref().unwrap())
        .is_err());
}

struct ProviderFixture {
    submissions: std::sync::atomic::AtomicUsize,
    unknown: bool,
    terminal_poll: bool,
}
#[async_trait::async_trait]
impl x_talk::meetings::providers::MeetingProvider for ProviderFixture {
    fn check_key(&self, _: &str) -> x_talk::domain::Result<()> {
        Ok(())
    }
    async fn upload(&self, _: &std::path::Path) -> x_talk::domain::Result<String> {
        Ok("https://fixture.invalid/audio".into())
    }
    async fn submit(&self, _: &str) -> x_talk::domain::Result<String> {
        self.submissions
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if self.unknown {
            return Err(x_talk::domain::AppError::new(
                "transcribe",
                "transport",
                "Response lost",
            ));
        }
        Ok("known-remote-id".into())
    }
    async fn poll(
        &self,
        remote: &str,
    ) -> x_talk::domain::Result<x_talk::meetings::providers::Poll> {
        assert_eq!(remote, "known-remote-id");
        if self.terminal_poll {
            return Err(x_talk::domain::AppError::new(
                "meeting_transcription",
                "provider_failed",
                "Synthetic terminal failure",
            ));
        }
        Ok(x_talk::meetings::providers::Poll::Complete(
            fixture().transcript.unwrap(),
        ))
    }
    async fn brief(
        &self,
        _: &Meeting,
    ) -> x_talk::domain::Result<x_talk::meetings::providers::GeneratedBrief> {
        Ok(x_talk::meetings::providers::GeneratedBrief {
            model: Some("fixture-brief".into()),
            content: BriefContent {
                title: "Generated title".into(),
                summary: "A saved summary.".into(),
                topics: vec![],
                decisions: vec![],
                actions: vec![],
                open_questions: vec![],
                suggested_follow_ups: vec![],
            },
        })
    }
}
fn queued_audio(store: &MeetingStore) -> String {
    let mut m = fixture();
    m.transcript = None;
    m.attempt = Attempt::queued(false);
    let path = store.audio_path(&m.id, "prepared.m4a").unwrap();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, b"fixture").unwrap();
    m.audio = Some(Audio {
        download: "prepared.m4a".into(),
        prepared: Some("prepared.m4a".into()),
        assets: vec!["prepared.m4a".into()],
    });
    let id = m.id.clone();
    store.insert(m).unwrap();
    id
}
#[tokio::test]
async fn pipeline_resumes_remote_id_without_duplicate_post_and_preserves_manual_title() {
    use x_talk::meetings::jobs::Worker;
    let dir = tempfile::tempdir().unwrap();
    let provider = std::sync::Arc::new(ProviderFixture {
        submissions: 0.into(),
        unknown: false,
        terminal_poll: false,
    });
    let store = MeetingStore::open(dir.path()).unwrap();
    let id = queued_audio(&store);
    let worker = Worker {
        store: store.clone(),
        provider: provider.clone(),
    };
    worker.step(store.get(&id).unwrap()).await.unwrap();
    assert_eq!(store.get(&id).unwrap().attempt.stage, Stage::Polling);
    drop(worker);
    drop(store);
    let store = MeetingStore::open(dir.path()).unwrap();
    let worker = Worker {
        store: store.clone(),
        provider: provider.clone(),
    };
    worker.step(store.get(&id).unwrap()).await.unwrap();
    let current = store.get(&id).unwrap();
    assert!(current.transcript.is_some());
    store
        .edit(&id, current.revision, |m| {
            m.title = "My title".into();
            m.manual_title = true;
            Ok(())
        })
        .unwrap();
    worker.step(store.get(&id).unwrap()).await.unwrap();
    let done = store.get(&id).unwrap();
    assert_eq!(done.attempt.stage, Stage::Complete);
    assert_eq!(done.title, "My title");
    assert!(!done.brief_stale());
    assert_eq!(
        provider
            .submissions
            .load(std::sync::atomic::Ordering::SeqCst),
        1
    );
}
#[tokio::test]
async fn ambiguous_paid_submission_is_never_automatically_replayed() {
    use x_talk::meetings::jobs::Worker;
    let dir = tempfile::tempdir().unwrap();
    let store = MeetingStore::open(dir.path()).unwrap();
    let id = queued_audio(&store);
    let provider = std::sync::Arc::new(ProviderFixture {
        submissions: 0.into(),
        unknown: true,
        terminal_poll: false,
    });
    let worker = Worker {
        store: store.clone(),
        provider: provider.clone(),
    };
    worker.step(store.get(&id).unwrap()).await.unwrap();
    assert_eq!(
        store.get(&id).unwrap().attempt.stage,
        Stage::SubmissionUnknown
    );
    assert!(!worker.step(store.get(&id).unwrap()).await.unwrap());
    assert_eq!(
        provider
            .submissions
            .load(std::sync::atomic::Ordering::SeqCst),
        1
    );
}

#[test]
fn normalization_preserves_unknown_confidence_and_rejects_out_of_bounds_turns() {
    use x_talk::meetings::providers::normalize;
    let mut raw = serde_json::json!({"speech_model_used":"model-actual","utterances":[
        {"speaker":"A","start":0,"end":2000,"text":"Hello","words":[{"text":"Hello","start":0,"end":1000,"confidence":null}]}
    ]});
    let t = normalize(&raw).unwrap();
    assert_eq!(t.model.as_deref(), Some("model-actual"));
    assert_eq!(t.turns[0].end, 2.0);
    assert_eq!(t.turns[0].words[0].confidence, None);
    raw["utterances"][0]["end"] = serde_json::json!(14_401_001);
    assert!(normalize(&raw).is_err());
}

#[test]
fn duplicate_audio_and_capture_warning_survive_retention_and_restart() {
    let dir = tempfile::tempdir().unwrap();
    let store = MeetingStore::open(dir.path()).unwrap();
    let mut first = fixture();
    first.audio_hash = Some("prepared-aac-v1:synthetic".into());
    first.capture_warning = Some("Synthetic capture discontinuity".into());
    store.insert(first.clone()).unwrap();
    let mut duplicate = fixture();
    duplicate.audio_hash = first.audio_hash.clone();
    assert!(store.insert(duplicate).is_err());
    drop(store);
    let store = MeetingStore::open(dir.path()).unwrap();
    let saved = store.get(&first.id).unwrap();
    assert_eq!(
        export::project(&saved).unwrap().capture_warning,
        first.capture_warning
    );
    assert!(!store.recoverable(&first.id).unwrap());
    assert_eq!(saved.audio_hash, first.audio_hash);
}

#[test]
fn explicit_retranscription_after_brief_failure_does_not_reuse_completed_remote_job() {
    let mut meeting = fixture();
    meeting.audio = Some(Audio {
        download: "original.wav".into(),
        prepared: None,
        assets: vec!["original.wav".into()],
    });
    meeting.attempt.stage = Stage::Failed;
    meeting.attempt.remote_id = Some("completed-remote-job".into());
    let generation = meeting.transcript.as_ref().unwrap().generation.clone();
    meeting.retry(true).unwrap();
    assert_eq!(meeting.attempt.stage, Stage::Queued);
    assert!(meeting.attempt.remote_id.is_none());
    assert!(meeting.attempt.replacement);
    assert_eq!(meeting.transcript.as_ref().unwrap().generation, generation);
    meeting.attempt.stage = Stage::Failed;
    meeting.attempt.remote_id = Some("unfinished-replacement".into());
    meeting.retry(true).unwrap();
    assert_eq!(meeting.attempt.stage, Stage::Polling);
    assert_eq!(
        meeting.attempt.remote_id.as_deref(),
        Some("unfinished-replacement")
    );
}

#[tokio::test]
async fn terminal_provider_job_requires_explicit_new_submission() {
    use x_talk::meetings::jobs::Worker;
    let dir = tempfile::tempdir().unwrap();
    let store = MeetingStore::open(dir.path()).unwrap();
    let id = queued_audio(&store);
    let provider = std::sync::Arc::new(ProviderFixture {
        submissions: 0.into(),
        unknown: false,
        terminal_poll: true,
    });
    let worker = Worker {
        store: store.clone(),
        provider: provider.clone(),
    };
    worker.step(store.get(&id).unwrap()).await.unwrap();
    worker.step(store.get(&id).unwrap()).await.unwrap();
    let failed = store.get(&id).unwrap();
    assert_eq!(failed.attempt.stage, Stage::Failed);
    assert!(failed.attempt.remote_id.is_none());
    assert!(!worker.step(failed.clone()).await.unwrap());
    assert_eq!(
        provider
            .submissions
            .load(std::sync::atomic::Ordering::SeqCst),
        1
    );
    store.edit(&id, failed.revision, |m| m.retry(true)).unwrap();
    worker.step(store.get(&id).unwrap()).await.unwrap();
    assert_eq!(
        provider
            .submissions
            .load(std::sync::atomic::Ordering::SeqCst),
        2
    );
}

#[test]
fn raw_capture_cleanup_waits_for_durable_compact_admission_and_is_repeatable() {
    let dir = tempfile::tempdir().unwrap();
    let store = MeetingStore::open(dir.path()).unwrap();
    let mut meeting = fixture();
    meeting.source = SourceKind::Recording;
    let directory = store.audio_path(&meeting.id, "microphone.f32").unwrap();
    std::fs::create_dir_all(directory.parent().unwrap()).unwrap();
    std::fs::write(&directory, b"raw recovery evidence").unwrap();
    assert!(store.cleanup_capture_working_files(&meeting.id).is_err());
    assert!(directory.exists());
    for name in ["prepared.m4a", "microphone.m4a"] {
        std::fs::write(
            store.audio_path(&meeting.id, name).unwrap(),
            b"saved compact audio",
        )
        .unwrap();
    }
    meeting.audio = Some(Audio {
        download: "prepared.m4a".into(),
        prepared: Some("prepared.m4a".into()),
        assets: vec!["prepared.m4a".into(), "microphone.m4a".into()],
    });
    store.insert(meeting.clone()).unwrap();
    // Simulate a crash immediately after admission, before raw-file cleanup.
    drop(store);
    let store = MeetingStore::open(dir.path()).unwrap();
    store.cleanup_capture_working_files(&meeting.id).unwrap();
    store.cleanup_capture_working_files(&meeting.id).unwrap();
    assert!(!directory.exists());
    assert!(store
        .audio_path(&meeting.id, "microphone.m4a")
        .unwrap()
        .exists());
    assert!(store.get(&meeting.id).unwrap().audio.is_some());
}

#[test]
fn lightweight_library_metadata_tracks_edits_and_upgrades_older_documents() {
    let dir = tempfile::tempdir().unwrap();
    let store = MeetingStore::open(dir.path()).unwrap();
    let meeting = fixture();
    store.insert(meeting.clone()).unwrap();
    assert_eq!(store.summaries().unwrap()[0].speakers, 3);
    let edited = store
        .edit(&meeting.id, 0, |m| {
            m.merge("A", "B")?;
            m.title = "Quoted \"meeting\"\nreview".into();
            Ok(())
        })
        .unwrap();
    let summary = store.summaries().unwrap().remove(0);
    assert_eq!(summary.speakers, 2);
    assert_eq!(summary.title, edited.title);
    assert_eq!(summary.revision, edited.revision);
    drop(store);
    // An earlier preview saved the same domain document without a list cache.
    let db = rusqlite::Connection::open(dir.path().join("meetings.sqlite3")).unwrap();
    let plan: String = db.query_row("EXPLAIN QUERY PLAN SELECT id,json_extract(document,'$._list') FROM meetings ORDER BY added_at DESC,id DESC", [], |row| row.get(3)).unwrap();
    assert!(
        plan.contains("meetings_list_summary"),
        "Library reads must use the metadata index: {plan}"
    );
    db.execute(
        "UPDATE meetings SET document=?1 WHERE id=?2",
        rusqlite::params![serde_json::to_string(&edited).unwrap(), edited.id],
    )
    .unwrap();
    drop(db);
    let store = MeetingStore::open(dir.path()).unwrap();
    let upgraded = store.summaries().unwrap().remove(0);
    assert_eq!(upgraded.revision, edited.revision);
    assert_eq!(upgraded.title, edited.title);
    assert_eq!(upgraded.speakers, 2);
    let exported: serde_json::Value =
        serde_json::from_str(&export::json(&store.get(&edited.id).unwrap()).unwrap()).unwrap();
    assert!(exported.get("_list").is_none());
    assert_eq!(
        exported["transcript"]["turns"].as_array().unwrap().len(),
        edited.transcript.unwrap().turns.len()
    );
}

#[test]
fn generated_citations_map_to_stable_ids_and_reject_invalid_references() {
    use x_talk::meetings::providers::{brief_response_schema, parse_generated_brief};
    let m = fixture();
    let transcript = m.transcript.as_ref().unwrap();
    let schema = brief_response_schema(transcript.turns.len());
    assert_eq!(
        schema["properties"]["topics"]["items"]["properties"]["segment_ids"]["items"]["maximum"],
        4
    );
    let mut response = serde_json::json!({"title":"Review","summary":"A summary","topics":[{"text":"Discussed plan","segment_ids":[1,4]}],"decisions":[],"actions":[],"open_questions":[],"suggested_follow_ups":[]});
    let brief = parse_generated_brief(&response.to_string(), transcript).unwrap();
    assert_eq!(brief.topics[0].segment_ids, vec!["a", "d"]);
    for refs in [
        serde_json::json!([0]),
        serde_json::json!([5]),
        serde_json::json!(["a"]),
        serde_json::json!([1.5]),
        serde_json::json!([]),
        serde_json::json!([1, 2, 3, 4]),
    ] {
        response["topics"][0]["segment_ids"] = refs;
        assert!(parse_generated_brief(&response.to_string(), transcript).is_err());
    }
}

#[test]
fn imports_are_visible_before_audio_and_recover_without_provider_work() {
    for stage in [Stage::Importing, Stage::Extracting, Stage::Converting] {
        let dir = tempfile::tempdir().unwrap();
        let store = MeetingStore::open(dir.path()).unwrap();
        let mut m = Meeting::new("Chosen title".into(), SourceKind::Video, None);
        m.attempt.stage = stage.clone();
        m.manual_title = true;
        m.call_at = "2026-10-01T12:00:00Z".into();
        store.insert(m.clone()).unwrap();
        assert_eq!(store.summaries().unwrap()[0].status, stage);
        assert!(store.pending().unwrap().is_empty());
        assert!(store.update_busy().unwrap());
        assert!(store.delete(&m.id, m.revision).is_err());
        let path = dir.path().join("audio").join(&m.id);
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("partial"), b"unfinished").unwrap();
        drop(store);
        let store = MeetingStore::open(dir.path()).unwrap();
        let recovered = store.get(&m.id).unwrap();
        assert_eq!(recovered.attempt.stage, Stage::Failed);
        assert!(recovered.attempt.error.unwrap().contains("interrupted"));
        assert!(!path.exists());
        assert!(store.pending().unwrap().is_empty());
        assert_eq!(
            store.summaries().unwrap()[0].call_at,
            "2026-10-01T12:00:00Z"
        );
    }
}

#[test]
fn completed_import_preserves_metadata_and_checks_duplicate_before_publication() {
    let dir = tempfile::tempdir().unwrap();
    let store = MeetingStore::open(dir.path()).unwrap();
    let mut m = Meeting::new("Chosen title".into(), SourceKind::Video, None);
    m.attempt.stage = Stage::Importing;
    m.manual_title = true;
    store.insert(m.clone()).unwrap();
    let audio = Audio {
        download: "prepared.m4a".into(),
        prepared: Some("prepared.m4a".into()),
        assets: vec!["prepared.m4a".into()],
    };
    let saved = store
        .complete_import(
            &m.id,
            &m.attempt.id,
            audio.clone(),
            "same-audio".into(),
            25.0,
        )
        .unwrap();
    assert_eq!(saved.attempt.stage, Stage::Queued);
    assert!(saved.manual_title);
    assert_eq!(saved.title, "Chosen title");
    let mut other = Meeting::new("Duplicate".into(), SourceKind::Video, None);
    other.attempt.stage = Stage::Importing;
    store.insert(other.clone()).unwrap();
    assert!(store
        .complete_import(
            &other.id,
            &other.attempt.id,
            audio,
            "same-audio".into(),
            25.0
        )
        .is_err());
    assert!(store.get(&other.id).unwrap().audio.is_none());
}
