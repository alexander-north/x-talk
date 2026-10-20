//! Explicit, billable, bounded synthetic meeting check. Credentials never enter app settings.
use std::{collections::BTreeMap, path::PathBuf, sync::Arc};
use x_talk::{
    credentials::CredentialStore,
    domain::{AppError, Result},
    meetings::{domain::*, jobs::Worker, store::MeetingStore},
    providers::Providers,
};
struct Keys(BTreeMap<String, String>);
impl CredentialStore for Keys {
    fn get(&self, p: &str) -> Result<String> {
        self.0
            .get(p)
            .cloned()
            .ok_or_else(|| AppError::input("Required live-test key is missing."))
    }
    fn set(&self, _: &str, _: &str) -> Result<()> {
        unreachable!()
    }
    fn delete(&self, _: &str) -> Result<()> {
        unreachable!()
    }
}
#[tokio::test]
#[ignore = "Billable small synthetic fixture; explicit keys-file and audio opt-in required"]
async fn assemblyai_and_luna_meeting() {
    let fixture = PathBuf::from(
        std::env::var_os("XTALK_MEETING_LIVE_AUDIO").expect("Synthetic fixture required"),
    );
    let keyfile = PathBuf::from(
        std::env::var_os("XTALK_MEETING_KEYS_FILE").expect("Explicit keys file required"),
    );
    let source = std::fs::read_to_string(keyfile).expect("Could not read authorized private keys");
    let mut keys = BTreeMap::new();
    for line in source.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let provider = match key.trim() {
            "ASSEMBLY_AI_KEY" | "ASSEMBLYAI_API_KEY" => "assemblyai",
            "OPENAI_API_KEY" | "OPENAI_KEY" => "openai",
            _ => continue,
        };
        keys.insert(
            provider.into(),
            value.trim().trim_matches(['\'', '"']).into(),
        );
    }
    let tools = x_talk::media::Ffmpeg {
        ffmpeg: "/usr/bin/ffmpeg".into(),
        ffprobe: "/usr/bin/ffprobe".into(),
    };
    let info = x_talk::meetings::media::inspect(&tools, &fixture)
        .await
        .unwrap();
    assert!(
        info.duration <= 60.0,
        "Live meeting checks are limited to sixty seconds."
    );
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.tmp/live-meeting-check");
    let store = MeetingStore::open(&root).unwrap();
    let id = if let Some(m) = store.list().unwrap().first() {
        m.id.clone()
    } else {
        let mut m = Meeting::new(
            "Synthetic proposal review".into(),
            SourceKind::Audio,
            Some("synthetic-dialogue.wav".into()),
        );
        let audio = store.audio_path(&m.id, "prepared.m4a").unwrap();
        std::fs::create_dir_all(audio.parent().unwrap()).unwrap();
        x_talk::meetings::media::prepare(&tools, &fixture, &audio)
            .await
            .unwrap();
        m.duration = Some(info.duration);
        m.audio = Some(Audio {
            download: "prepared.m4a".into(),
            prepared: Some("prepared.m4a".into()),
            assets: vec!["prepared.m4a".into()],
        });
        let id = m.id.clone();
        store.insert(m).unwrap();
        id
    };
    let worker = Worker {
        store: store.clone(),
        provider: Arc::new(Providers::new(Arc::new(Keys(keys))).unwrap()),
    };
    for _ in 0..120 {
        let m = store.get(&id).unwrap();
        if m.attempt.stage == Stage::Complete {
            let exported = x_talk::meetings::export::json(&m).unwrap();
            std::fs::write(root.join("synthetic-meeting.json"), exported).unwrap();
            assert!(m.transcript.as_ref().is_some_and(|t| !t.turns.is_empty()));
            assert!(m.brief.content.is_some());
            println!("AssemblyAI transcript and Luna structured brief verified; saved synthetic JSON receipt.");
            return;
        }
        assert!(
            !matches!(
                m.attempt.stage,
                Stage::Failed | Stage::SubmissionUnknown | Stage::WaitingForKeys
            ),
            "Live processing stopped safely: {:?}; {:?}",
            m.attempt.stage,
            m.attempt.error
        );
        worker.step(m).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
    panic!("Polling deadline exceeded. The remote ID is saved; rerun resumes polling without another submission.");
}
