//! Explicitly authorized, billable comparison. Private fixtures/results stay in .tmp.
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use x_talk::{
    credentials::{CredentialStore, NativeCredentials},
    domain::*,
    media::{Ffmpeg, MediaProcessor, PreparedAudio},
    providers::{Providers, TranscriptionProvider},
    realtime::{ElevenLabs, LiveSession},
};

#[derive(Deserialize)]
struct Manifest {
    clips: Vec<Clip>,
    compression: bool,
    silence_removal: bool,
}
#[derive(Deserialize)]
struct Clip {
    name: String,
    pcm: PathBuf,
    original: PathBuf,
    duration: f64,
}
#[derive(Serialize)]
struct Measurement {
    clip: String,
    round: u32,
    mode: String,
    audio_seconds: f64,
    preparation_ms: u64,
    request_ms: u64,
    stop_to_final_ms: u64,
    first_preview_ms: Option<u64>,
    text: Option<String>,
    error: Option<AppError>,
}
fn persist(dir: &std::path::Path, row: &Measurement) {
    std::fs::write(
        dir.join(format!("{}-{}-{}.json", row.clip, row.round, row.mode)),
        serde_json::to_vec_pretty(row).unwrap(),
    )
    .unwrap();
    println!("clip={} round={} mode={} success={} stop_to_final_ms={} first_preview_ms={:?} error_category={}",row.clip,row.round,row.mode,row.error.is_none(),row.stop_to_final_ms,row.first_preview_ms,row.error.as_ref().map(|e|e.category.as_str()).unwrap_or("none"));
}

#[test]
#[ignore = "Billable: explicitly authorize live ElevenLabs calls and set XTALK_COMPARE_MANIFEST to permitted fixtures."]
fn compare_elevenlabs() {
    let path = PathBuf::from(
        std::env::var_os("XTALK_COMPARE_MANIFEST").expect("Explicit fixture manifest required"),
    );
    let manifest: Manifest = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert!(manifest.clips.len() <= 4);
    let dir = path.parent().unwrap();
    let rounds: u32 = std::env::var("XTALK_COMPARE_ROUNDS")
        .unwrap_or("1".into())
        .parse()
        .unwrap();
    assert!((1..=2).contains(&rounds));
    let first_round: u32 = std::env::var("XTALK_COMPARE_FIRST_ROUND")
        .unwrap_or("1".into())
        .parse()
        .unwrap();
    assert!((1..=rounds).contains(&first_round));
    let filter = std::env::var("XTALK_COMPARE_CLIP").ok();
    let keys: Arc<dyn CredentialStore> = Arc::new(NativeCredentials);
    assert!(
        keys.get("elevenlabs").is_ok(),
        "An installed ElevenLabs credential is required"
    );
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let provider = runtime.block_on(async { Providers::new(keys.clone()).unwrap() });
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let media = Ffmpeg {
        ffmpeg: root.join("binaries/ffmpeg.exe"),
        ffprobe: root.join("binaries/ffprobe.exe"),
    };
    let config = Transcription {
        provider: "elevenlabs".into(),
        model: "scribe_v2".into(),
        prompt: String::new(),
        temperature: None,
    };
    let mut failures = 0;
    for round in first_round..=rounds {
        for clip in &manifest.clips {
            if filter.as_ref().is_some_and(|name| name != &clip.name) {
                continue;
            }
            assert!(clip.duration > 0. && clip.duration <= 60.);
            // Reverse order on the second round to reduce a simple warm-connection bias.
            let modes = if round == 1 {
                ["realtime", "batch-identical", "batch-installed"]
            } else {
                ["batch-installed", "batch-identical", "realtime"]
            };
            for mode in modes {
                let mut row = Measurement {
                    clip: clip.name.clone(),
                    round,
                    mode: mode.into(),
                    audio_seconds: clip.duration,
                    preparation_ms: 0,
                    request_ms: 0,
                    stop_to_final_ms: 0,
                    first_preview_ms: None,
                    text: None,
                    error: None,
                };
                let result = if mode == "realtime" {
                    let mut wav = hound::WavReader::open(&clip.pcm).unwrap();
                    let spec = wav.spec();
                    assert_eq!(spec.bits_per_sample, 16);
                    assert_eq!(spec.channels, 1);
                    assert!(x_talk::realtime::supported_rate(spec.sample_rate));
                    let first = Arc::new(Mutex::new(None));
                    let first_copy = first.clone();
                    let began = Instant::now();
                    let (session, mut feed) = LiveSession::start(
                        Arc::new(ElevenLabs { keys: keys.clone() }),
                        spec.sample_rate,
                        spec.channels,
                        config.clone(),
                        Arc::new(move |p| {
                            if !p.text.is_empty() {
                                first_copy
                                    .lock()
                                    .unwrap()
                                    .get_or_insert(began.elapsed().as_millis() as u64);
                            }
                        }),
                    )
                    .unwrap();
                    let chunk = spec.sample_rate as usize / 10;
                    let samples: Vec<i16> = wav.samples::<i16>().map(|s| s.unwrap()).collect();
                    let mut count = 0;
                    for samples in samples.chunks(chunk) {
                        count += samples.len();
                        // Make audio available only after its capture time has elapsed.
                        std::thread::sleep(
                            Duration::from_secs_f64(count as f64 / spec.sample_rate as f64)
                                .saturating_sub(began.elapsed()),
                        );
                        for sample in samples {
                            feed.push(*sample as f32 / i16::MAX as f32);
                        }
                    }
                    let stopped = Instant::now();
                    feed.flush();
                    drop(feed);
                    let outcome = session.finish(stopped);
                    row.request_ms = began.elapsed().as_millis() as u64;
                    row.stop_to_final_ms = outcome.finalize_ms;
                    row.first_preview_ms = *first.lock().unwrap();
                    outcome.result
                } else {
                    runtime.block_on(async {
                        let mut settings = Settings {
                            transcription: config.clone(),
                            ..Settings::default()
                        };
                        settings.compression = manifest.compression;
                        settings.silence_removal = manifest.silence_removal;
                        let snapshot = Snapshot::capture(&settings, Preset::seed());
                        let started = Instant::now();
                        let audio = if mode == "batch-identical" {
                            PreparedAudio {
                                path: clip.pcm.clone(),
                                bytes: std::fs::metadata(&clip.pcm).unwrap().len(),
                                mime: "audio/wav",
                            }
                        } else {
                            media
                                .prepare(
                                    &clip.original,
                                    &dir.join(format!("prepare-{}-{round}", clip.name)),
                                    &snapshot,
                                )
                                .await?
                        };
                        row.preparation_ms = started.elapsed().as_millis() as u64;
                        let sent = Instant::now();
                        let result = provider
                            .transcribe(&audio, &Job::new(id(), "import", snapshot, false), &|_| {
                                Ok(())
                            })
                            .await;
                        row.request_ms = sent.elapsed().as_millis() as u64;
                        row.stop_to_final_ms = started.elapsed().as_millis() as u64;
                        result
                    })
                };
                match result {
                    Ok(text) => row.text = Some(text),
                    Err(error) => {
                        failures += 1;
                        row.error = Some(error);
                    }
                }
                persist(dir, &row);
            }
        }
    }
    assert_eq!(
        failures, 0,
        "Some live requests failed; inspect the private comparison receipts."
    );
}
