//! Opt-in, billable checks using synthetic/explicitly permitted fixtures only.
#[test]
#[ignore = "Billable: explicitly authorize ElevenLabs and supply XTALK_LIVE_AUDIO (mono/stereo PCM WAV, at most 60 seconds)."]
fn realtime_elevenlabs() {
    use x_talk::realtime::{ElevenLabs, LiveSession};
    let fixture = PathBuf::from(
        std::env::var("XTALK_LIVE_AUDIO").expect("Explicitly permitted WAV fixture required"),
    );
    let mut wav = hound::WavReader::open(&fixture).unwrap();
    let spec = wav.spec();
    assert!(x_talk::realtime::supported_rate(spec.sample_rate));
    assert!(matches!(spec.channels, 1 | 2));
    assert!(
        wav.duration() <= spec.sample_rate * 60,
        "Use a bounded fixture for the billable check"
    );
    assert_eq!(spec.sample_format, hound::SampleFormat::Int);
    assert_eq!(spec.bits_per_sample, 16, "Supply 16-bit PCM WAV");
    let config = Transcription {
        provider: "elevenlabs".into(),
        model: "scribe_v2".into(),
        prompt: std::env::var("XTALK_LIVE_CONTEXT").unwrap_or_default(),
        temperature: None,
    };
    let (session, mut feed) = LiveSession::start(
        Arc::new(ElevenLabs {
            keys: Arc::new(keys()),
        }),
        spec.sample_rate,
        spec.channels,
        config,
        Arc::new(|_| {}),
    )
    .unwrap();
    let began = std::time::Instant::now();
    let chunk = spec.sample_rate as usize * spec.channels as usize / 10;
    for (index, sample) in wav.samples::<i16>().enumerate() {
        feed.push(sample.unwrap() as f32 / i16::MAX as f32);
        if (index + 1) % chunk == 0 {
            let due = std::time::Duration::from_secs_f64(
                (index + 1) as f64 / spec.sample_rate as f64 / spec.channels as f64,
            );
            std::thread::sleep(due.saturating_sub(began.elapsed()));
        }
    }
    let stopped = std::time::Instant::now();
    feed.flush();
    drop(feed);
    let outcome = session.finish(stopped);
    let text = outcome
        .result
        .expect("Real-time transcription must finalize");
    assert!(!text.trim().is_empty());
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.tmp");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("live-elevenlabs-realtime-transcript.txt"), text).unwrap();
    println!(
        "provider=elevenlabs mode=realtime stop_to_final_ms={}",
        outcome.finalize_ms
    );
}
use std::{collections::HashMap, path::PathBuf, sync::Arc};
use x_talk::{
    credentials::CredentialStore,
    domain::*,
    media::{Ffmpeg, MediaProcessor},
    providers::{Providers, TranscriptionProvider, TransformationProvider},
};
struct Keys(HashMap<String, String>);
impl CredentialStore for Keys {
    fn get(&self, p: &str) -> Result<String> {
        self.0.get(p).cloned().ok_or_else(|| {
            AppError::new(
                "credentials",
                "missing_key",
                format!("No test key configured for {p}."),
            )
        })
    }
    fn set(&self, _: &str, _: &str) -> Result<()> {
        unreachable!()
    }
    fn delete(&self, _: &str) -> Result<()> {
        unreachable!()
    }
}
fn keys() -> Keys {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.env");
    let text = std::fs::read_to_string(path).expect("Private .env required for live checks");
    let mut keys = HashMap::new();
    for line in text.lines() {
        if let Some((k, v)) = line.split_once('=') {
            let provider = match k.trim() {
                "OPEN_AI" | "OPENAI_API_KEY" => "openai",
                "ELEVEN_LABS" | "ELEVENLABS_API_KEY" => "elevenlabs",
                "ASSEMBLYAI_API_KEY" => "assemblyai",
                "DEEPGRAM_API_KEY" => "deepgram",
                "ANTHROPIC_API_KEY" => "anthropic",
                _ => continue,
            };
            keys.insert(provider.into(), v.trim().trim_matches(['\'', '"']).into());
        }
    }
    Keys(keys)
}
#[tokio::test]
#[ignore = "Billable: configure private keys and XTALK_LIVE_AUDIO, then explicitly request --ignored."]
async fn transcribe_synthetic() {
    let provider = std::env::var("XTALK_LIVE_PROVIDER").unwrap_or("openai".into());
    let fixture =
        PathBuf::from(std::env::var_os("XTALK_LIVE_AUDIO").expect("XTALK_LIVE_AUDIO required"));
    let ffmpeg = Ffmpeg {
        ffmpeg: std::env::var_os("XTALK_FFMPEG")
            .map(PathBuf::from)
            .unwrap_or("/usr/bin/ffmpeg".into()),
        ffprobe: std::env::var_os("XTALK_FFPROBE")
            .map(PathBuf::from)
            .unwrap_or("/usr/bin/ffprobe".into()),
    };
    let scratch = tempfile::tempdir().unwrap();
    let mut settings = Settings::default();
    let model = catalog()
        .into_iter()
        .find(|m| m.provider == provider && m.stage == "transcription")
        .unwrap();
    settings.transcription.prompt = std::env::var("XTALK_LIVE_CONTEXT").unwrap_or_default();
    settings.transcription.provider = provider.clone();
    settings.transcription.model = model.id.into();
    let snapshot = Snapshot::capture(&settings, Preset::seed());
    let audio = ffmpeg
        .prepare(&fixture, scratch.path(), &snapshot)
        .await
        .unwrap();
    let j = Job::new(id(), "import", snapshot, false);
    let adapters = Providers::new(Arc::new(keys())).unwrap();
    let started = std::time::Instant::now();
    match adapters.transcribe(&audio, &j, &|_| Ok(())).await {
        Ok(text) => {
            assert!(text.len() > 30, "Expected synthetic speech transcript");
            let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.tmp");
            std::fs::write(dir.join(format!("live-{provider}-transcript.txt")), &text).unwrap();
            println!(
                "provider={provider} success=true bytes={} transcript_chars={} elapsed_seconds={}",
                audio.bytes,
                text.chars().count(),
                started.elapsed().as_secs()
            );
        }
        Err(e) => panic!(
            "provider={provider} category={} stage={} message={}",
            e.category, e.stage, e.message
        ),
    }
}
#[tokio::test]
#[ignore = "Billable: explicitly run with private configured keys."]
async fn cleanup_synthetic() {
    let provider = std::env::var("XTALK_LIVE_PROVIDER").unwrap_or("openai".into());
    let mut p = Preset::seed();
    let m = catalog()
        .into_iter()
        .find(|m| m.provider == provider && m.stage == "transformation")
        .unwrap();
    p.provider = provider.clone();
    p.model = m.id.into();
    let input = if let Some(path) = std::env::var_os("XTALK_LIVE_TEXT") {
        std::fs::read_to_string(path).unwrap()
    } else {
        "I uh think the budget is maybe twenty thousand dollars split across two quarters. Do not ship before Wednesday. Can you just, uh, check the logging and things like that?".into()
    };
    let api = Providers::new(Arc::new(keys())).unwrap();
    let start = std::time::Instant::now();
    match api.transform(&input, &p).await {
        Ok(output) => {
            assert!(!output.trim().is_empty());
            if input.len() < 500 {
                for protected in ["maybe", "twenty thousand", "not", "Wednesday"] {
                    assert!(output.contains(protected), "Cleanup lost protected meaning");
                }
            }
            std::fs::write(
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.tmp/live-cleanup-output.txt"),
                &output,
            )
            .unwrap();
            println!("provider={provider} success=true input_chars={} output_chars={} elapsed_seconds={}",input.chars().count(),output.chars().count(),start.elapsed().as_secs());
        }
        Err(e) => panic!(
            "provider={provider} category={} stage={} message={}",
            e.category, e.stage, e.message
        ),
    }
}
