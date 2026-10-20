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
