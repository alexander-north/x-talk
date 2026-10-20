use super::domain::MAX_SECONDS;
use crate::{
    domain::{AppError, Result},
    media::Ffmpeg,
};
use serde::{Deserialize, Serialize};
use std::{path::Path, time::Duration};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Inspection {
    pub duration: f64,
    pub audio_streams: usize,
    pub video: bool,
    pub channels: u32,
}
#[derive(Deserialize)]
struct Probe {
    streams: Vec<Stream>,
    format: Format,
}
#[derive(Deserialize)]
struct Stream {
    codec_type: Option<String>,
    channels: Option<u32>,
}
#[derive(Deserialize)]
struct Format {
    duration: Option<String>,
}
pub async fn content_hash(path: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};
    use tokio::io::AsyncReadExt;
    let mut file = tokio::fs::File::open(path).await?;
    let mut buffer = vec![0u8; 64 * 1024];
    let mut hasher = Sha256::new();
    loop {
        let count = file.read(&mut buffer).await?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(format!("prepared-aac-v1:{:x}", hasher.finalize()))
}

pub async fn inspect(tools: &Ffmpeg, path: &Path) -> Result<Inspection> {
    if !path.is_file() {
        return Err(AppError::input("Choose an audio or video file."));
    }
    let bytes = tools
        .run(
            &tools.ffprobe,
            &[
                "-v".into(),
                "error".into(),
                "-protocol_whitelist".into(),
                "file,pipe".into(),
                "-show_entries".into(),
                "stream=codec_type,channels:format=duration".into(),
                "-of".into(),
                "json".into(),
                path.as_os_str().into(),
            ],
            Duration::from_secs(60),
        )
        .await?;
    let probe: Probe = serde_json::from_slice(&bytes)?;
    let audio: Vec<_> = probe
        .streams
        .iter()
        .filter(|s| s.codec_type.as_deref() == Some("audio"))
        .collect();
    let first = audio
        .first()
        .ok_or_else(|| AppError::input("This file has no audio stream."))?;
    let channels = first.channels.unwrap_or(0);
    if !(1..=2).contains(&channels) {
        return Err(AppError::input(
            "Meeting imports currently support mono or stereo audio.",
        ));
    }
    let duration = probe
        .format
        .duration
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|v| v.is_finite() && *v > 0.0 && *v <= MAX_SECONDS + 0.1)
        .ok_or_else(|| {
            AppError::input("Choose a recording with a known duration of up to four hours.")
        })?;
    Ok(Inspection {
        duration,
        audio_streams: audio.len(),
        channels,
        video: probe
            .streams
            .iter()
            .any(|s| s.codec_type.as_deref() == Some("video")),
    })
}

/// Stream from the external input directly. Video is neither staged nor retained.
/// Timestamp-aware resampling preserves silence and initial audio offsets.
pub async fn prepare(tools: &Ffmpeg, input: &Path, output: &Path) -> Result<Inspection> {
    let source = inspect(tools, input).await?;
    tools
        .run(
            &tools.ffmpeg,
            &[
                "-nostdin".into(),
                "-v".into(),
                "error".into(),
                "-threads".into(),
                "2".into(),
                "-protocol_whitelist".into(),
                "file,pipe".into(),
                "-i".into(),
                input.as_os_str().into(),
                "-map".into(),
                "0:a:0".into(),
                "-vn".into(),
                "-sn".into(),
                "-dn".into(),
                "-map_metadata".into(),
                "-1".into(),
                "-af".into(),
                "aresample=async=1000:first_pts=0".into(),
                "-ar".into(),
                "48000".into(),
                "-c:a".into(),
                "aac".into(),
                "-b:a".into(),
                "128k".into(),
                "-threads".into(),
                "2".into(),
                "-f".into(),
                "ipod".into(),
                "-n".into(),
                output.as_os_str().into(),
            ],
            Duration::from_secs(4 * 3600),
        )
        .await?;
    let prepared = inspect(tools, output).await?;
    if prepared.video
        || prepared.channels != source.channels
        || (prepared.duration - source.duration).abs() > 1.0
    {
        return Err(AppError::input("Prepared audio duration or channels do not match the source. No transcription was submitted."));
    }
    if output.metadata()?.len() > 2_200_000_000 {
        return Err(AppError::input(
            "Prepared audio exceeds AssemblyAI's upload limit.",
        ));
    }
    std::fs::OpenOptions::new()
        .write(true)
        .open(output)?
        .sync_all()?;
    Ok(prepared)
}

#[cfg(feature = "desktop")]
pub async fn mix_capture(
    tools: &Ffmpeg,
    directory: &Path,
    journal: &super::capture::Journal,
) -> Result<()> {
    let mut args: Vec<std::ffi::OsString> = ["-nostdin", "-v", "error", "-threads", "2"]
        .into_iter()
        .map(Into::into)
        .collect();
    for track in &journal.tracks {
        args.extend([
            "-f".into(),
            "f32le".into(),
            "-ar".into(),
            track.rate.to_string().into(),
            "-ac".into(),
            track.channels.to_string().into(),
            "-i".into(),
            directory.join(&track.filename).into_os_string(),
        ]);
    }
    let filter = if journal.tracks.len() == 2 {
        "[0:a]aresample=48000,aformat=channel_layouts=stereo[a];[1:a]aresample=48000,aformat=channel_layouts=stereo[b];[a][b]amix=inputs=2:duration=longest:normalize=0,alimiter=limit=0.95,apad[out]"
    } else {
        "[0:a]aresample=48000,aformat=channel_layouts=stereo,apad[out]"
    };
    args.extend([
        "-filter_complex".into(),
        filter.into(),
        "-map".into(),
        "[out]".into(),
        "-t".into(),
        format!("{:.6}", journal.elapsed).into(),
        "-c:a".into(),
        "aac".into(),
        "-b:a".into(),
        "128k".into(),
        "-threads".into(),
        "2".into(),
        "-f".into(),
        "ipod".into(),
        "-y".into(),
        directory.join("prepared.m4a").into_os_string(),
    ]);
    tools
        .run(&tools.ffmpeg, &args, Duration::from_secs(4 * 3600))
        .await?;
    let info = inspect(tools, &directory.join("prepared.m4a")).await?;
    if (info.duration - journal.elapsed).abs() > 1.0 {
        return Err(AppError::input(
            "Meeting mix duration did not match capture. Source audio is preserved.",
        ));
    }
    std::fs::OpenOptions::new()
        .write(true)
        .open(directory.join("prepared.m4a"))?
        .sync_all()?;
    Ok(())
}
