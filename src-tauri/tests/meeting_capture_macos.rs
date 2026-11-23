//! macOS meeting capture (Core Audio IO procs and a system-audio process tap).
//!
//! The non-ignored tests need no audio permission and no audio hardware, so
//! they run on headless GitHub macOS runners: they only exercise device
//! resolution and failure reporting. Tests that open real devices are ignored:
//! CI runners cannot grant microphone or system-audio-recording permission.
//! Run them on a Mac from a terminal that has both permissions, for example:
//! `cargo test --manifest-path src-tauri/Cargo.toml --features desktop --test meeting_capture_macos -- --ignored --test-threads=1`
#![cfg(all(target_os = "macos", feature = "desktop"))]
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use x_talk::{
    capture::{play, PlaybackControl},
    media::Ffmpeg,
    meetings::{
        capture::{Monitor, Options, Session},
        media::mix_capture,
    },
};

fn options(microphone: Option<&str>, output: Option<&str>) -> Options {
    Options {
        microphone: microphone.is_some(),
        system: output.is_some(),
        microphone_device: microphone.filter(|s| !s.is_empty()).map(Into::into),
        output_device: output.filter(|s| !s.is_empty()).map(Into::into),
    }
}

fn tool(name: &str, variable: &str) -> PathBuf {
    if let Some(path) = std::env::var_os(variable) {
        return path.into();
    }
    let bundled = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("binaries")
        .join(name);
    if bundled.metadata().is_ok_and(|m| m.len() > 1_000_000) {
        return bundled;
    }
    ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"]
        .iter()
        .map(|dir| PathBuf::from(dir).join(name))
        .find(|path| path.is_file())
        .unwrap_or_else(|| panic!("{name} not found; set {variable}"))
}
fn tools() -> Ffmpeg {
    Ffmpeg {
        ffmpeg: tool("ffmpeg", "XTALK_FFMPEG"),
        ffprobe: tool("ffprobe", "XTALK_FFPROBE"),
    }
}

#[test]
fn missing_microphone_fails_start_without_a_journal() {
    let dir = tempfile::tempdir().unwrap();
    let capture = dir.path().join("capture");
    let error = Session::start(
        uuid::Uuid::new_v4().to_string(),
        capture.clone(),
        options(Some("coreaudio:xtalk-test-missing-microphone"), None),
    )
    .err()
    .expect("an unknown microphone must not start");
    assert!(!error.message.is_empty());
    assert!(
        !capture.join("capture.json").exists(),
        "no recoverable journal for a capture that never started"
    );
}

#[test]
fn missing_output_fails_start_without_a_journal() {
    let dir = tempfile::tempdir().unwrap();
    let capture = dir.path().join("capture");
    let error = Session::start(
        uuid::Uuid::new_v4().to_string(),
        capture.clone(),
        options(None, Some("coreaudio:xtalk-test-missing-output")),
    )
    .err()
    .expect("an unknown output device must not start");
    assert!(error.message.contains("output device"), "{}", error.message);
    assert!(
        !capture.join("capture.json").exists(),
        "no recoverable journal for a capture that never started"
    );
}

#[test]
fn monitor_reports_missing_devices_inline() {
    let monitor = Monitor::start(options(
        Some("coreaudio:xtalk-test-missing-microphone"),
        None,
    ))
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        monitor.renew();
        let status = monitor.status();
        if status.error.is_some() || Instant::now() > deadline {
            break status;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let error = status.error.expect("monitor error is reported");
    assert!(error.starts_with("Could not monitor the selected devices."));
    assert!(!status.active);
}

fn tone(path: &std::path::Path, seconds: u32) {
    let mut wav = hound::WavWriter::create(
        path,
        hound::WavSpec {
            channels: 1,
            sample_rate: 24000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .unwrap();
    for n in 0..24000 * seconds {
        let sample = ((n as f32 * 440.0 * std::f32::consts::TAU / 24000.0).sin() * 1500.0) as i16;
        wav.write_sample(sample).unwrap();
    }
    wav.finalize().unwrap();
}
async fn play_tone(tools: &Ffmpeg, path: &std::path::Path) {
    play(
        tools,
        path,
        Arc::new(AtomicBool::new(false)),
        Arc::new(PlaybackControl::new(0.0, false)),
        0.0,
    )
    .await
    .unwrap();
}
fn samples(path: &std::path::Path) -> Vec<f32> {
    std::fs::read(path)
        .unwrap()
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect()
}

#[tokio::test]
#[ignore = "uses the Mac's microphone and output and needs both audio permissions; coordinate with the owner"]
async fn macos_dual_source_stop_saves_timed_audio() {
    let dir = tempfile::tempdir().unwrap();
    let tools = tools();
    let wav = dir.path().join("synthetic.wav");
    tone(&wav, 2);
    let session = Session::start(
        uuid::Uuid::new_v4().to_string(),
        dir.path().join("capture"),
        options(Some(""), Some("")),
    )
    .unwrap();
    play_tone(&tools, &wav).await;
    let mute_start = session.status().elapsed;
    session.microphone_muted.store(true, Ordering::Release);
    tokio::time::sleep(Duration::from_secs(1)).await;
    let mute_end = session.status().elapsed;
    session.microphone_muted.store(false, Ordering::Release);
    tokio::time::sleep(Duration::from_secs(2)).await;
    let status = session.status();
    assert!(
        status.active && status.error.is_none(),
        "{:?}",
        status.error
    );
    let journal = session.finish().unwrap();
    println!("Capture warning: {:?}", journal.warning);
    assert_eq!(journal.tracks.len(), 2);
    let capture = dir.path().join("capture");
    for track in &journal.tracks {
        assert!((8000..=96000).contains(&track.rate) && (1..=2).contains(&track.channels));
        let data = samples(&capture.join(&track.filename));
        let frames = data.len() / track.channels as usize;
        let duration = frames as f64 / track.rate as f64;
        assert!(
            (duration - journal.elapsed).abs() < 0.01,
            "{} lasts {duration}, session {}",
            track.filename,
            journal.elapsed
        );
        if track.microphone {
            let at = |t: f64| (t * track.rate as f64) as usize * track.channels as usize;
            assert!(
                data[at(mute_start + 0.15)..at(mute_end - 0.15)]
                    .iter()
                    .all(|v| *v == 0.0),
                "muted microphone audio was retained"
            );
        } else {
            assert!(
                data.iter().any(|v| v.abs() > 0.001),
                "the system tap did not capture the synthetic tone; check System Audio Recording permission"
            );
        }
    }
    mix_capture(&tools, &capture, &journal).await.unwrap();
}

#[test]
#[ignore = "opens the Mac's default devices and needs both audio permissions; coordinate with the owner"]
fn macos_device_switches_keep_the_timeline_aligned() {
    let dir = tempfile::tempdir().unwrap();
    let session = Session::start(
        uuid::Uuid::new_v4().to_string(),
        dir.path().to_path_buf(),
        options(Some(""), Some("")),
    )
    .unwrap();
    std::thread::sleep(Duration::from_millis(1100));
    assert!(session
        .change_device(true, Some("coreaudio:xtalk-test-missing".into()))
        .is_err());
    assert!(session.status().active, "a failed switch keeps recording");
    session.system_muted.store(true, Ordering::Release);
    session.change_device(true, None).unwrap();
    for _ in 0..5 {
        // Repeated tap switches (each creates and destroys a tap and an
        // aggregate device) keep recording.
        session.change_device(false, None).unwrap();
    }
    std::thread::sleep(Duration::from_millis(600));
    let status = session.status();
    assert!(status.active && status.system_muted, "{:?}", status.error);
    let journal = session.finish().unwrap();
    for track in journal.tracks {
        let data = samples(&dir.path().join(&track.filename));
        let frames = data.len() / track.channels as usize;
        assert!((frames as f64 / track.rate as f64 - journal.elapsed).abs() < 0.01);
    }
}
