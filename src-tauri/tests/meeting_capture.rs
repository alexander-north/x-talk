//! Explicit real-device acceptance. Never part of unattended default tests.
#![cfg(all(windows, feature = "desktop"))]
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use x_talk::{
    capture::{play, PlaybackControl},
    media::Ffmpeg,
    meetings::{
        capture::{Options, Session},
        media::mix_capture,
    },
};

#[tokio::test]
#[ignore = "uses the Windows microphone and output; coordinate with the owner"]
async fn windows_dual_source_stop_saves_timed_audio() {
    let dir = tempfile::tempdir().unwrap();
    let seconds: u64 = std::env::var("XTALK_MEETING_CAPTURE_SECONDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(6);
    assert!((6..=14400).contains(&seconds));
    let session = Session::start(
        uuid::Uuid::new_v4().to_string(),
        dir.path().to_path_buf(),
        Options {
            microphone: true,
            system: true,
            microphone_device: None,
            output_device: None,
        },
    )
    .unwrap();
    let tools = Ffmpeg {
        ffmpeg: std::env::var_os("XTALK_FFMPEG")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("binaries/ffmpeg.exe")
            }),
        ffprobe: std::env::var_os("XTALK_FFPROBE")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("binaries/ffprobe.exe")
            }),
    };
    let tone = dir.path().join("synthetic.wav");
    let mut wav = hound::WavWriter::create(
        &tone,
        hound::WavSpec {
            channels: 1,
            sample_rate: 24000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .unwrap();
    for n in 0..48000 {
        let sample = ((n as f32 * 440.0 * std::f32::consts::TAU / 24000.0).sin() * 1500.0) as i16;
        wav.write_sample(sample).unwrap();
    }
    wav.finalize().unwrap();
    let start = std::time::Instant::now();
    play(
        &tools,
        &tone,
        Arc::new(AtomicBool::new(false)),
        Arc::new(PlaybackControl::new(0.0, false)),
        0.0,
    )
    .await
    .unwrap();
    let mute_start = session.status().elapsed;
    session.microphone_muted.store(true, Ordering::Release);
    tokio::time::sleep(Duration::from_secs(1)).await;
    let mute_end = session.status().elapsed;
    // Long runs exercise the live microphone clock while keeping ambient speech out of retained audio.
    session
        .microphone_muted
        .store(seconds > 60, Ordering::Release);
    session.system_muted.store(seconds > 60, Ordering::Release);
    while start.elapsed().as_secs() < seconds.saturating_sub(2) {
        let status = session.status();
        assert!(status.error.is_none(), "{:?}", status.error);
        assert!(status.active);
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    session.system_muted.store(false, Ordering::Release);
    play(
        &tools,
        &tone,
        Arc::new(AtomicBool::new(false)),
        Arc::new(PlaybackControl::new(0.0, false)),
        0.0,
    )
    .await
    .unwrap();
    let journal = session.finish().unwrap();
    assert!(journal.elapsed >= seconds as f64);
    assert!(journal.elapsed < seconds as f64 + 3.0);
    assert_eq!(journal.tracks.len(), 2);
    println!("Capture quality warning: {:?}", journal.warning);
    for t in &journal.tracks {
        let frames = std::fs::metadata(dir.path().join(&t.filename))
            .unwrap()
            .len()
            / (t.channels as u64 * 4);
        let duration = frames as f64 / t.rate as f64;
        assert!(
            (duration - journal.elapsed).abs() < 0.1,
            "source duration {duration}, session {}",
            journal.elapsed
        );
    }
    for track in &journal.tracks {
        use std::io::{Read, Seek, SeekFrom};
        let mut file = std::fs::File::open(dir.path().join(&track.filename)).unwrap();
        if track.microphone {
            let start =
                ((mute_start + 0.15) * track.rate as f64) as u64 * track.channels as u64 * 4;
            let end = ((mute_end - 0.15) * track.rate as f64) as u64 * track.channels as u64 * 4;
            file.seek(SeekFrom::Start(start)).unwrap();
            let mut bytes = vec![0; (end - start) as usize];
            file.read_exact(&mut bytes).unwrap();
            assert!(
                bytes.iter().all(|v| *v == 0),
                "Muted microphone audio was retained"
            );
        } else {
            let mut bytes = vec![0; track.rate as usize * track.channels as usize * 4 * 3];
            file.read_exact(&mut bytes).unwrap();
            assert!(
                bytes
                    .chunks_exact(4)
                    .any(|b| f32::from_le_bytes(b.try_into().unwrap()).abs() > 0.001),
                "Loopback did not capture the synthetic tone"
            );
            file.seek(SeekFrom::End(
                -(track.rate as i64 * track.channels as i64 * 4 * 2),
            ))
            .unwrap();
            file.read_exact(&mut bytes[..track.rate as usize * track.channels as usize * 4 * 2])
                .unwrap();
            assert!(
                bytes[..track.rate as usize * track.channels as usize * 4 * 2]
                    .chunks_exact(4)
                    .any(|b| f32::from_le_bytes(b.try_into().unwrap()).abs() > 0.001),
                "Loopback did not capture the ending signal after the idle interval"
            );
        }
    }
    for track in &journal.tracks {
        let data = std::fs::read(dir.path().join(&track.filename)).unwrap();
        let samples: Vec<f32> = data
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        let peak = samples.iter().fold(0.0_f32, |p, v| p.max(v.abs()));
        let mono: Vec<f64> = samples
            .chunks_exact(track.channels as usize)
            .map(|frame| frame.iter().map(|v| *v as f64).sum::<f64>() / track.channels as f64)
            .collect();
        let tone_peak = mono
            .chunks((track.rate / 2) as usize)
            .map(|window| {
                let (mut real, mut imag) = (0.0_f64, 0.0_f64);
                for (i, value) in window.iter().enumerate() {
                    let phase = std::f64::consts::TAU * 440.0 * i as f64 / track.rate as f64;
                    real += value * phase.cos();
                    imag += value * phase.sin();
                }
                2.0 * (real * real + imag * imag).sqrt() / window.len() as f64
            })
            .fold(0.0_f64, f64::max);
        println!(
            "Synthetic source check: microphone={}, peak={peak:.6}, tone_440_peak={tone_peak:.6}",
            track.microphone
        );
    }
    mix_capture(&tools, dir.path(), &journal).await.unwrap();
    let info = x_talk::meetings::media::inspect(&tools, &dir.path().join("prepared.m4a"))
        .await
        .unwrap();
    assert!((info.duration - journal.elapsed).abs() < 0.1);
    assert!(!info.video);
    println!(
        "Native dual-source Stop verified: {:.3}s, two synchronized source tracks, audio-only mix.",
        journal.elapsed
    );
}

#[tokio::test]
#[ignore = "plays a quiet tone through Windows output; coordinate with the owner"]
async fn monitor_levels_match_saved_samples_without_monitor_files() {
    use x_talk::meetings::capture::Monitor;
    let root = tempfile::tempdir().unwrap();
    let tone = root.path().join("level-tone.wav");
    let mut wav = hound::WavWriter::create(
        &tone,
        hound::WavSpec {
            channels: 1,
            sample_rate: 24000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .unwrap();
    for n in 0..24000 {
        wav.write_sample(
            ((n as f32 * 440.0 * std::f32::consts::TAU / 24000.0).sin() * 1500.0) as i16,
        )
        .unwrap();
    }
    wav.finalize().unwrap();
    let tools = Ffmpeg {
        ffmpeg: std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("binaries/ffmpeg.exe"),
        ffprobe: std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("binaries/ffprobe.exe"),
    };
    let options = Options {
        microphone: false,
        system: true,
        microphone_device: None,
        output_device: None,
    };
    let monitor = Monitor::start(options.clone()).unwrap();
    let playback = play(
        &tools,
        &tone,
        Arc::new(AtomicBool::new(false)),
        Arc::new(PlaybackControl::new(0.0, false)),
        0.0,
    );
    let sampling = async {
        let mut peak = 0.0_f32;
        for _ in 0..15 {
            monitor.renew();
            let state = monitor.status();
            assert!(state.error.is_none(), "{:?}", state.error);
            peak = peak.max(state.system_level);
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        peak
    };
    let (played, preview_peak) = tokio::join!(playback, sampling);
    played.unwrap();
    assert!(preview_peak > 0.0001, "No output signal in monitor");
    // The lease must release devices if the frontend stops renewing it.
    tokio::time::sleep(Duration::from_millis(2300)).await;
    assert!(!monitor.status().active);
    assert!(monitor.expired());
    drop(monitor);
    assert_eq!(
        std::fs::read_dir(root.path()).unwrap().count(),
        1,
        "Preview saved audio"
    );
    let session = Session::start(
        uuid::Uuid::new_v4().to_string(),
        root.path().join("capture"),
        options,
    )
    .unwrap();
    let playback = play(
        &tools,
        &tone,
        Arc::new(AtomicBool::new(false)),
        Arc::new(PlaybackControl::new(0.0, false)),
        0.0,
    );
    let sampling = async {
        let mut peak = 0.0_f32;
        for _ in 0..15 {
            peak = peak.max(session.status().system_level);
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        peak
    };
    let (played, recording_peak) = tokio::join!(playback, sampling);
    played.unwrap();
    let journal = session.finish().unwrap();
    let bytes = std::fs::read(
        root.path()
            .join("capture")
            .join(&journal.tracks[0].filename),
    )
    .unwrap();
    let saved_peak = bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()).abs())
        .fold(0.0_f32, f32::max);
    assert!(
        (recording_peak - saved_peak).abs() < 0.002,
        "Meter {recording_peak} / saved {saved_peak}"
    );
    assert!(
        (preview_peak - saved_peak).abs() < 0.005,
        "Preview {preview_peak} / saved {saved_peak}"
    );
    println!("Synthetic tone peaks: input={:.6}, preview={preview_peak:.6}, recording={recording_peak:.6}, saved={saved_peak:.6}",1500.0/32768.0);
}

#[test]
#[ignore = "uses Windows microphone and output; coordinate with the owner"]
fn compatible_device_switches_preserve_recording_and_mute() {
    let dir = tempfile::tempdir().unwrap();
    let session = Session::start(
        uuid::Uuid::new_v4().to_string(),
        dir.path().to_path_buf(),
        Options {
            microphone: true,
            system: true,
            microphone_device: None,
            output_device: None,
        },
    )
    .unwrap();
    std::thread::sleep(Duration::from_millis(1100));
    let prefixes: Vec<_> = ["microphone.f32", "system.f32"]
        .into_iter()
        .map(|name| (name, std::fs::read(dir.path().join(name)).unwrap()))
        .collect();
    assert!(!prefixes[0].1.is_empty());
    assert!(session
        .change_device(true, Some("missing-endpoint".into()))
        .is_err());
    assert!(session.status().active);
    session.microphone_muted.store(true, Ordering::Release);
    session.system_muted.store(true, Ordering::Release);
    session.change_device(true, None).unwrap();
    session.change_device(false, None).unwrap();
    std::thread::sleep(Duration::from_millis(600));
    let status = session.status();
    assert!(status.active && status.microphone_muted && status.system_muted);
    let journal = session.finish().unwrap();
    assert_eq!(journal.tracks.len(), 2);
    for (name, prefix) in prefixes {
        assert!(std::fs::read(dir.path().join(name))
            .unwrap()
            .starts_with(&prefix));
    }
    for track in journal.tracks {
        let bytes = std::fs::read(dir.path().join(track.filename)).unwrap();
        let frames = bytes.len() / (track.channels as usize * 4);
        assert!((frames as f64 / track.rate as f64 - journal.elapsed).abs() < 0.002);
        let tail = &bytes[bytes.len() - track.rate as usize / 4 * track.channels as usize * 4..];
        assert!(tail.iter().all(|byte| *byte == 0));
    }
}
