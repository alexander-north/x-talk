use std::{
    io::{Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};
use x_talk::{
    media::Ffmpeg,
    meetings::media::{content_hash, inspect, prepare},
};
fn tools() -> Ffmpeg {
    let binary = |name: &str| {
        if let Some(path) = std::env::var_os(if name == "ffmpeg" {
            "XTALK_FFMPEG"
        } else {
            "XTALK_FFPROBE"
        }) {
            return PathBuf::from(path);
        }
        if cfg!(windows) {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("binaries/{name}.exe"))
        } else {
            PathBuf::from(format!("/usr/bin/{name}"))
        }
    };
    Ffmpeg {
        ffmpeg: binary("ffmpeg"),
        ffprobe: binary("ffprobe"),
    }
}
fn sparse_wav(path: &Path, seconds: u32) {
    let bytes = seconds * 16000 * 2;
    let mut file = std::fs::File::create(path).unwrap();
    file.write_all(b"RIFF").unwrap();
    file.write_all(&(bytes + 36).to_le_bytes()).unwrap();
    file.write_all(b"WAVEfmt ").unwrap();
    file.write_all(&16u32.to_le_bytes()).unwrap();
    file.write_all(&1u16.to_le_bytes()).unwrap();
    file.write_all(&1u16.to_le_bytes()).unwrap();
    file.write_all(&16000u32.to_le_bytes()).unwrap();
    file.write_all(&32000u32.to_le_bytes()).unwrap();
    file.write_all(&2u16.to_le_bytes()).unwrap();
    file.write_all(&16u16.to_le_bytes()).unwrap();
    file.write_all(b"data").unwrap();
    file.write_all(&bytes.to_le_bytes()).unwrap();
    file.set_len(44 + bytes as u64).unwrap();
    // Distinct beginning/end signals expose accidental truncation and silence removal.
    for at in [44, 44 + bytes as u64 - 32000] {
        file.seek(SeekFrom::Start(at)).unwrap();
        for n in 0..16000 {
            file.write_all(&(((n as f32 * 0.17).sin() * 2000.0) as i16).to_le_bytes())
                .unwrap();
        }
    }
}
#[tokio::test]
async fn video_extracts_first_audio_stream_and_preserves_stereo() {
    let dir = tempfile::tempdir().unwrap();
    let source =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/meeting-two-tracks.mkv");
    let before = std::fs::read(&source).unwrap();
    let info = inspect(&tools(), &source).await.unwrap();
    assert!(info.video);
    assert_eq!(info.audio_streams, 2);
    assert_eq!(info.channels, 2);
    let output = dir.path().join("audio.m4a");
    let info = prepare(&tools(), &source, &output).await.unwrap();
    assert!(!info.video);
    assert_eq!(info.audio_streams, 1);
    assert_eq!(info.channels, 2);
    assert_eq!(before, std::fs::read(&source).unwrap());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    assert!(content_hash(&output)
        .await
        .unwrap()
        .starts_with("prepared-aac-v1:"));
}
#[tokio::test]
async fn meeting_media_preserves_silence_and_rejects_over_four_hours() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.wav");
    sparse_wav(&source, 10);
    let info = prepare(&tools(), &source, &dir.path().join("audio.m4a"))
        .await
        .unwrap();
    assert!((info.duration - 10.0).abs() < 0.1);
    sparse_wav(&source, 14401);
    assert!(inspect(&tools(), &source).await.is_err());
}
#[tokio::test]
#[ignore = "long bounded-memory media acceptance"]
async fn four_hour_meeting_preparation_retains_the_complete_timeline() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("four-hours.wav");
    sparse_wav(&source, 14400);
    let info = prepare(&tools(), &source, &dir.path().join("audio.m4a"))
        .await
        .unwrap();
    assert!((info.duration - 14400.0).abs() < 0.1);
    println!(
        "Four-hour preparation verified: {:.3}s, {} channel(s).",
        info.duration, info.channels
    );
}

#[cfg(feature = "desktop")]
async fn check_capture_archive(seconds: u64) {
    use x_talk::meetings::{
        capture::{Journal, Track},
        media::finalize_capture,
    };
    let dir = tempfile::tempdir().unwrap();
    let tracks = vec![
        Track {
            filename: "microphone.f32".into(),
            rate: 16000,
            channels: 1,
            microphone: true,
        },
        Track {
            filename: "system.f32".into(),
            rate: 48000,
            channels: 2,
            microphone: false,
        },
    ];
    let mut raw_bytes = 0;
    for track in &tracks {
        let mut file = std::fs::File::create(dir.path().join(&track.filename)).unwrap();
        let length = seconds * track.rate as u64 * track.channels as u64 * 4;
        file.set_len(length).unwrap();
        let tone: Vec<u8> = (0..track.rate)
            .flat_map(|n| {
                let value = (n as f32 * 0.17).sin() * 0.1;
                (0..track.channels).flat_map(move |_| value.to_le_bytes())
            })
            .collect();
        for offset in [0, length - tone.len() as u64] {
            file.seek(SeekFrom::Start(offset)).unwrap();
            file.write_all(&tone).unwrap();
        }
        raw_bytes += length;
    }
    let journal = Journal {
        id: x_talk::domain::id(),
        created: x_talk::domain::now(),
        tracks,
        elapsed: seconds as f64,
        ready: true,
        discarded: false,
        warning: None,
    };
    let assets = finalize_capture(&tools(), dir.path(), &journal)
        .await
        .unwrap();
    assert_eq!(assets, ["prepared.m4a", "microphone.m4a", "system.m4a"]);
    let mut archived_bytes = 0;
    for asset in &assets {
        let info = inspect(&tools(), &dir.path().join(asset)).await.unwrap();
        assert!(!info.video);
        assert!((info.duration - seconds as f64).abs() < 0.1);
        assert_eq!(info.channels, if asset == "microphone.m4a" { 1 } else { 2 });
        archived_bytes += std::fs::metadata(dir.path().join(asset)).unwrap().len();
    }
    for track in &journal.tracks {
        assert!(
            dir.path().join(&track.filename).exists(),
            "Recovery inputs must remain until admission"
        );
    }
    assert!(archived_bytes < raw_bytes / 2);
    println!("Capture archive: {seconds}s, {raw_bytes} raw bytes, {archived_bytes} retained bytes; mono/stereo and timeline preserved.");
}
#[cfg(feature = "desktop")]
#[tokio::test]
async fn compact_capture_assets_preserve_sources_and_recovery_inputs() {
    check_capture_archive(5).await;
}
#[cfg(feature = "desktop")]
#[tokio::test]
#[ignore = "long local capture-archive acceptance; no microphone or providers"]
async fn two_hour_capture_archive_preserves_the_complete_timeline() {
    check_capture_archive(7200).await;
}

#[tokio::test]
async fn compatible_import_copies_packets_and_pcm_without_encoding() {
    use x_talk::meetings::media::{import_audio, ImportPlan};
    let dir = tempfile::tempdir().unwrap();
    let input =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/meeting-two-tracks.mkv");
    let mut phases = Vec::new();
    let result = import_audio(&tools(), &input, dir.path(), |stage| {
        phases.push(stage);
        Ok(())
    })
    .await
    .unwrap();
    assert_eq!(result.plan, ImportPlan::Extract("m4a", "ipod"));
    assert_eq!(result.info.audio_streams, 1);
    assert!(!result.info.video);
    async fn packets(path: &Path) -> Vec<u8> {
        tools()
            .run(
                &tools().ffprobe,
                &[
                    "-v".into(),
                    "error".into(),
                    "-select_streams".into(),
                    "a:0".into(),
                    "-show_packets".into(),
                    "-show_data_hash".into(),
                    "sha256".into(),
                    "-show_entries".into(),
                    "packet=data_hash".into(),
                    "-of".into(),
                    "json".into(),
                    path.as_os_str().into(),
                ],
                std::time::Duration::from_secs(30),
            )
            .await
            .unwrap()
    }
    assert_eq!(
        packets(&input).await,
        packets(&dir.path().join(&result.filename)).await,
        "Encoded audio packets changed"
    );
    assert_eq!(phases, vec![x_talk::meetings::domain::Stage::Extracting]);
    let wav = dir.path().join("source.wav");
    sparse_wav(&wav, 10);
    let result = import_audio(&tools(), &wav, dir.path(), |_| Ok(()))
        .await
        .unwrap();
    assert_eq!(result.plan, ImportPlan::Copy("wav"));
    assert_eq!(
        std::fs::read(&wav).unwrap(),
        std::fs::read(dir.path().join(result.filename)).unwrap()
    );
}
#[test]
fn fast_import_requires_supported_codec_timing_and_upload_size() {
    use x_talk::meetings::media::{import_plan, ImportPlan, Inspection};
    let mut info = Inspection {
        duration: 100.0,
        audio_streams: 1,
        video: true,
        channels: 2,
        codec: "aac".into(),
        format: "mov,mp4,m4a,3gp,3g2,mj2".into(),
        start: 0.0,
        streams: 2,
    };
    assert_eq!(
        import_plan(&info, 3_000_000_000),
        ImportPlan::Extract("m4a", "ipod"),
        "Large video size is not audio upload size"
    );
    info.start = 1.5;
    assert_eq!(import_plan(&info, 1000), ImportPlan::Convert);
    info.start = 0.0;
    info.codec = "unsupported".into();
    assert_eq!(import_plan(&info, 1000), ImportPlan::Convert);
    info.video = false;
    info.streams = 1;
    info.format = "wav".into();
    info.codec = "pcm_alaw".into();
    assert_eq!(
        import_plan(&info, 1000),
        ImportPlan::Convert,
        "Do not bypass the bundled playback codec support"
    );
    for (format, codec, ext) in [
        ("mp3", "mp3", "mp3"),
        ("flac", "flac", "flac"),
        ("ogg", "opus", "ogg"),
        ("ogg", "vorbis", "ogg"),
        ("aac", "aac", "aac"),
        ("wav", "pcm_s16le", "wav"),
    ] {
        info.format = format.into();
        info.codec = codec.into();
        assert_eq!(import_plan(&info, 1000), ImportPlan::Copy(ext));
        assert_eq!(import_plan(&info, 2_200_000_001), ImportPlan::Convert);
    }
}
#[tokio::test]
async fn offset_audio_falls_back_to_conversion() {
    use x_talk::meetings::media::{import_audio, ImportPlan};
    let dir = tempfile::tempdir().unwrap();
    let wav = dir.path().join("source.wav");
    sparse_wav(&wav, 5);
    let delayed = dir.path().join("offset.m4a");
    tools()
        .run(
            &tools().ffmpeg,
            &[
                "-v".into(),
                "error".into(),
                "-itsoffset".into(),
                "1".into(),
                "-i".into(),
                wav.as_os_str().into(),
                "-c:a".into(),
                "aac".into(),
                delayed.as_os_str().into(),
            ],
            std::time::Duration::from_secs(30),
        )
        .await
        .unwrap();
    let input = inspect(&tools(), &delayed).await.unwrap();
    assert!(input.start > 0.5);
    let result = import_audio(&tools(), &delayed, dir.path(), |_| Ok(()))
        .await
        .unwrap();
    assert_eq!(result.plan, ImportPlan::Convert);
    assert!((result.info.duration - input.duration).abs() < 0.1);
    assert!(result.info.start.abs() < 0.05);
    assert!(!dir.path().join("extracted.m4a").exists());
}

#[tokio::test]
#[ignore = "local one-hour synthetic audio import benchmark; no providers"]
async fn benchmark_one_hour_audio_import() {
    use x_talk::meetings::media::{import_audio, ImportPlan};
    let dir = tempfile::tempdir().unwrap();
    let wav = dir.path().join("hour.wav");
    sparse_wav(&wav, 3600);
    let source = dir.path().join("hour.m4a");
    prepare(&tools(), &wav, &source).await.unwrap();
    let started = std::time::Instant::now();
    let fast = import_audio(&tools(), &source, dir.path(), |_| Ok(()))
        .await
        .unwrap();
    let fast_elapsed = started.elapsed();
    assert_eq!(fast.plan, ImportPlan::Extract("m4a", "ipod"));
    let started = std::time::Instant::now();
    let normalized = prepare(&tools(), &source, &dir.path().join("normalized.m4a"))
        .await
        .unwrap();
    assert!((fast.info.duration - normalized.duration).abs() < 0.1);
    println!("One-hour synthetic AAC track: direct extraction {:?}; prior conversion {:?}; input {} bytes; audio {} bytes",fast_elapsed,started.elapsed(),source.metadata().unwrap().len(),dir.path().join(fast.filename).metadata().unwrap().len());
}

// Uses the pinned Windows FFmpeg 8 build; older system FFmpeg loops this fixture differently.
#[cfg(windows)]
#[tokio::test]
#[ignore = "local one-hour synthetic import benchmark; no providers"]
async fn benchmark_one_hour_video_import() {
    use x_talk::meetings::media::{import_audio, ImportPlan};
    let dir = tempfile::tempdir().unwrap();
    let input =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/meeting-two-tracks.mkv");
    let source = dir.path().join("hour.mp4");
    tools()
        .run(
            &tools().ffmpeg,
            &[
                "-v".into(),
                "error".into(),
                "-stream_loop".into(),
                "-1".into(),
                "-i".into(),
                input.as_os_str().into(),
                "-map".into(),
                "0:v:0".into(),
                "-map".into(),
                "0:a:0".into(),
                "-c".into(),
                "copy".into(),
                "-t".into(),
                "3600".into(),
                "-avoid_negative_ts".into(),
                "make_zero".into(),
                "-f".into(),
                "ipod".into(),
                source.as_os_str().into(),
            ],
            std::time::Duration::from_secs(120),
        )
        .await
        .unwrap();
    let started = std::time::Instant::now();
    let fast = import_audio(&tools(), &source, dir.path(), |_| Ok(()))
        .await
        .unwrap();
    let fast_elapsed = started.elapsed();
    assert_eq!(fast.plan, ImportPlan::Extract("m4a", "ipod"));
    let started = std::time::Instant::now();
    let normalized = prepare(&tools(), &source, &dir.path().join("normalized.m4a"))
        .await
        .unwrap();
    assert!((fast.info.duration - normalized.duration).abs() < 1.0);
    println!("One-hour synthetic video: direct extraction {:?}; prior conversion {:?}; input {} bytes; audio {} bytes",fast_elapsed,started.elapsed(),source.metadata().unwrap().len(),dir.path().join(fast.filename).metadata().unwrap().len());
}
