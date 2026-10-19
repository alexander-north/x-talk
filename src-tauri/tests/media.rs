use std::path::Path;
use x_talk::{
    domain::*,
    media::{Ffmpeg, MediaProcessor},
};
fn tools() -> Ffmpeg {
    Ffmpeg {
        ffmpeg: std::env::var_os("XTALK_FFMPEG")
            .map(Into::into)
            .unwrap_or_else(|| {
                if cfg!(windows) {
                    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("binaries/ffmpeg.exe")
                } else {
                    "/usr/bin/ffmpeg".into()
                }
            }),
        ffprobe: std::env::var_os("XTALK_FFPROBE")
            .map(Into::into)
            .unwrap_or_else(|| {
                if cfg!(windows) {
                    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                        .join("binaries/ffprobe.exe")
                } else {
                    "/usr/bin/ffprobe".into()
                }
            }),
    }
}
fn fixture(path: &Path, parts: &[(f64, f32)]) {
    let mut w = hound::WavWriter::create(
        path,
        hound::WavSpec {
            channels: 1,
            sample_rate: 24000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .unwrap();
    let mut t = 0;
    for (seconds, amplitude) in parts {
        for _ in 0..(*seconds * 24000.) as usize {
            let v = (t as f32 * 440. * std::f32::consts::TAU / 24000.).sin() * amplitude;
            w.write_sample((v * 32767.) as i16).unwrap();
            t += 1;
        }
    }
    w.finalize().unwrap();
}
fn snap() -> Snapshot {
    Snapshot::capture(&Settings::default(), Preset::seed())
}
#[tokio::test]
async fn probe_checks_actual_media_and_source_limits() {
    let d = tempfile::tempdir().unwrap();
    let file = d.path().join("wrong-extension.bin");
    fixture(&file, &[(1., 0.1)]);
    let info = tools().probe(&file).await.unwrap();
    assert!((info.duration - 1.).abs() < 0.001);
    let corrupt = d.path().join("corrupt.wav");
    std::fs::write(&corrupt, b"not audio").unwrap();
    assert!(tools().probe(&corrupt).await.is_err());
    let big = d.path().join("huge.wav");
    let f = std::fs::File::create(&big).unwrap();
    f.set_len(1_073_741_825).unwrap();
    assert!(tools()
        .probe(&big)
        .await
        .unwrap_err()
        .message
        .contains("1 GiB"));
}
#[tokio::test]
async fn silence_filter_keeps_quiet_speech_and_short_pauses_and_original() {
    let d = tempfile::tempdir().unwrap();
    let file = d.path().join("original.wav");
    fixture(
        &file,
        &[
            (0.5, 0.003),
            (0.3, 0.),
            (0.5, 0.003),
            (3., 0.),
            (0.5, 0.003),
        ],
    );
    let original = std::fs::read(&file).unwrap();
    let mut snapshot = snap();
    snapshot.compression = false;
    snapshot.silence_removal = true;
    let prepared = tools()
        .prepare(&file, &d.path().join("upload"), &snapshot)
        .await
        .unwrap();
    let r = hound::WavReader::open(prepared.path).unwrap();
    let seconds = r.duration() as f64 / r.spec().sample_rate as f64;
    assert!(
        (1.8..3.7).contains(&seconds),
        "Long silence was not shortened conservatively: {seconds}"
    );
    let input = hound::WavReader::new(std::io::Cursor::new(original.clone()))
        .unwrap()
        .into_samples::<i16>()
        .take(28800)
        .map(|s| s.unwrap())
        .collect::<Vec<_>>();
    let output = hound::WavReader::open(d.path().join("upload/upload.wav"))
        .unwrap()
        .into_samples::<i16>()
        .take(28800)
        .map(|s| s.unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        input, output,
        "Quiet speech and the short pause before long silence must stay unchanged"
    );
    assert_eq!(std::fs::read(file).unwrap(), original);
}
#[tokio::test]
async fn silent_input_never_reaches_provider() {
    let d = tempfile::tempdir().unwrap();
    let file = d.path().join("silent.wav");
    fixture(&file, &[(2., 0.)]);
    for compression in [false, true] {
        for silence_removal in [false, true] {
            let mut snapshot = snap();
            snapshot.compression = compression;
            snapshot.silence_removal = silence_removal;
            let e = tools()
                .prepare(
                    &file,
                    &d.path().join(format!("{compression}-{silence_removal}")),
                    &snapshot,
                )
                .await
                .unwrap_err();
            assert_eq!(
                e.category, "no_speech",
                "compression={compression}, silence_removal={silence_removal}"
            );
        }
    }
    assert!(file.exists());
}
#[tokio::test]
async fn compression_uses_bounded_derivative_and_preserves_original() {
    let d = tempfile::tempdir().unwrap();
    let file = d.path().join("signal.wav");
    fixture(&file, &[(2., 0.3)]);
    let original = std::fs::read(&file).unwrap();
    let result = tools()
        .prepare(&file, &d.path().join("upload"), &snap())
        .await
        .unwrap();
    assert_eq!(result.mime, "audio/mp4");
    assert!(result.bytes < 40_000);
    assert_eq!(std::fs::read(&file).unwrap(), original);
}

#[tokio::test]
async fn import_duration_boundary_accepts_one_hour_and_rejects_longer_audio() {
    use std::io::Write;
    let d = tempfile::tempdir().unwrap();
    for seconds in [3599u32, 3600, 3601] {
        // Sparse PCM fixture exercises real container probing without allocating an hour in RAM.
        let path = d.path().join(format!("{seconds}.wav"));
        let data_bytes = seconds * 8000 * 2;
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(b"RIFF").unwrap();
        f.write_all(&(data_bytes + 36).to_le_bytes()).unwrap();
        f.write_all(b"WAVEfmt ").unwrap();
        f.write_all(&16u32.to_le_bytes()).unwrap();
        f.write_all(&1u16.to_le_bytes()).unwrap();
        f.write_all(&1u16.to_le_bytes()).unwrap();
        f.write_all(&8000u32.to_le_bytes()).unwrap();
        f.write_all(&16000u32.to_le_bytes()).unwrap();
        f.write_all(&2u16.to_le_bytes()).unwrap();
        f.write_all(&16u16.to_le_bytes()).unwrap();
        f.write_all(b"data").unwrap();
        f.write_all(&data_bytes.to_le_bytes()).unwrap();
        f.set_len(u64::from(data_bytes) + 44).unwrap();
        drop(f);
        let result = tools().probe(&path).await;
        if seconds <= 3600 {
            assert_eq!(result.unwrap().duration, f64::from(seconds));
        } else {
            assert!(result.unwrap_err().message.contains("60 minute"));
        }
    }
}

#[tokio::test]
async fn playback_decoder_emits_interleaved_float_pcm_with_bundled_tools() {
    let d = tempfile::tempdir().unwrap();
    let input = d.path().join("original.wav");
    fixture(&input, &[(1., 0.1)]);
    let child = tools().decode_playback(&input, 48000, 2).await.unwrap();
    let output = child.wait_with_output().await.unwrap();
    assert!(output.status.success(), "Playback decoder failed");
    assert_eq!(output.stdout.len(), 48000 * 2 * 4);
    assert!(output
        .stdout
        .chunks_exact(4)
        .any(|sample| { f32::from_le_bytes(sample.try_into().unwrap()).abs() > 0.01 }));
}

#[tokio::test]
async fn playback_seek_decodes_only_the_remaining_audio_and_validates_position() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("seek.wav");
    fixture(&path, &[(3., 0.3)]);
    let media = tools();
    let mut child = media.decode_playback_at(&path, 24000, 1, 2.).await.unwrap();
    use tokio::io::AsyncReadExt;
    let mut bytes = Vec::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_end(&mut bytes)
        .await
        .unwrap();
    assert!(child.wait().await.unwrap().success());
    assert!((bytes.len() as i64 - 24000 * 4).abs() < 100);
    assert!(media
        .decode_playback_at(&path, 24000, 1, f64::NAN)
        .await
        .is_err());
    assert!(media
        .decode_playback_at(&path, 24000, 1, -1.)
        .await
        .is_err());
}

#[tokio::test]
async fn shared_decode_preserves_unfiltered_validation_for_every_upload_mode() {
    let d = tempfile::tempdir().unwrap();
    let source = d.path().join("stereo-float.wav");
    let mut writer = hound::WavWriter::create(
        &source,
        hound::WavSpec {
            channels: 2,
            sample_rate: 48000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )
    .unwrap();
    for i in 0..(48000 * 5) {
        let sample = if (48000..48000 * 4).contains(&i) {
            0.
        } else {
            (i as f32 * 440. * std::f32::consts::TAU / 48000.).sin() * 0.03
        };
        writer.write_sample(sample).unwrap();
        writer.write_sample(sample * 0.5).unwrap();
    }
    writer.finalize().unwrap();
    let original = std::fs::read(&source).unwrap();
    let mut reference = None;
    for compression in [false, true] {
        for silence_removal in [false, true] {
            let mut snapshot = snap();
            snapshot.compression = compression;
            snapshot.silence_removal = silence_removal;
            let dir = d.path().join(format!("{compression}-{silence_removal}"));
            let output = tools().prepare(&source, &dir, &snapshot).await.unwrap();
            let mut check = hound::WavReader::open(dir.join("decoded.wav")).unwrap();
            assert_eq!(check.spec().channels, 1);
            assert_eq!(check.spec().sample_rate, 24000);
            assert_eq!(check.duration(), 5 * 24000);
            let samples = check
                .samples::<i16>()
                .map(|s| s.unwrap())
                .collect::<Vec<_>>();
            if let Some(expected) = &reference {
                assert_eq!(&samples, expected);
            } else {
                reference = Some(samples);
            }
            let info = tools().probe(&output.path).await.unwrap();
            if silence_removal {
                assert!(
                    (2.0..4.0).contains(&info.duration),
                    "compression={compression}: duration={}",
                    info.duration
                );
            } else {
                assert!((4.9..5.2).contains(&info.duration));
            }
            assert_eq!(
                output.mime,
                if compression {
                    "audio/mp4"
                } else {
                    "audio/wav"
                }
            );
        }
    }
    assert_eq!(std::fs::read(source).unwrap(), original);
}
