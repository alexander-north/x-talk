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
