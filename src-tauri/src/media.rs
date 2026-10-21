use crate::domain::*;
use async_trait::async_trait;
use serde::Deserialize;
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::process::Command;
#[derive(Clone, Debug)]
pub struct AudioInfo {
    pub duration: f64,
    pub bytes: u64,
}
#[derive(Clone, Debug)]
pub struct PreparedAudio {
    pub path: PathBuf,
    pub bytes: u64,
    pub mime: &'static str,
}
#[async_trait]
pub trait MediaProcessor: Send + Sync {
    async fn probe(&self, path: &Path) -> Result<AudioInfo>;
    async fn prepare(&self, path: &Path, dir: &Path, snapshot: &Snapshot) -> Result<PreparedAudio>;
}
#[derive(Clone)]
pub struct Ffmpeg {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
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
    format_name: Option<String>,
}
impl Ffmpeg {
    pub async fn run(
        &self,
        exe: &Path,
        args: &[std::ffi::OsString],
        deadline: Duration,
    ) -> Result<Vec<u8>> {
        use tokio::io::AsyncReadExt;
        let mut cmd = Command::new(exe);
        cmd.args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        #[cfg(windows)]
        cmd.creation_flags(0x08000000);
        let mut child = cmd.spawn().map_err(|_| {
            AppError::new(
                "media",
                "tool_missing",
                "Bundled media tool could not start. Reinstall x-talk.",
            )
        })?;
        #[cfg(windows)]
        let _guard = ChildJob::attach(&child)?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| AppError::new("media", "process", "Media tool has no output stream."))?;
        let task = async {
            let mut output = Vec::new();
            stdout.take(1024 * 1024).read_to_end(&mut output).await?;
            let status = child.wait().await?;
            if !status.success() {
                return Err(AppError::new(
                    "media",
                    "invalid_audio",
                    "Audio could not be decoded. Choose a supported, uncorrupted audio-only file.",
                ));
            }
            Ok(output)
        };
        tokio::time::timeout(deadline, task)
            .await
            .map_err(|_| AppError::new("media", "timeout", "Audio preparation timed out."))?
    }
    pub async fn decode_playback(
        &self,
        path: &Path,
        rate: u32,
        channels: u16,
    ) -> Result<tokio::process::Child> {
        self.decode_playback_at(path, rate, channels, 0.).await
    }
    pub async fn decode_playback_at(
        &self,
        path: &Path,
        rate: u32,
        channels: u16,
        start: f64,
    ) -> Result<tokio::process::Child> {
        // Public commands enforce their workflow limit; the shared decoder also serves meetings.
        if !start.is_finite() || !(0. ..=crate::meetings::domain::MAX_SECONDS).contains(&start) {
            return Err(AppError::input("Invalid playback position."));
        }
        let mut c = Command::new(&self.ffmpeg);
        c.args([
            "-nostdin",
            "-v",
            "error",
            "-protocol_whitelist",
            "file,pipe",
        ])
        .arg("-ss")
        .arg(format!("{start:.6}"))
        .arg("-i")
        .arg(path)
        .args(["-vn", "-f", "f32le", "-ac"])
        .arg(channels.to_string())
        .arg("-ar")
        .arg(rate.to_string())
        .arg("pipe:1")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
        #[cfg(windows)]
        c.creation_flags(0x08000000);
        Ok(c.spawn()?)
    }
}
#[async_trait]
impl MediaProcessor for Ffmpeg {
    async fn probe(&self, path: &Path) -> Result<AudioInfo> {
        let bytes = tokio::fs::metadata(path).await?.len();
        if bytes > 1_073_741_824 {
            return Err(AppError::input(
                "Audio source exceeds the 1 GiB import limit.",
            ));
        }
        let args = vec![
            "-v".into(),
            "error".into(),
            "-protocol_whitelist".into(),
            "file,pipe".into(),
            "-show_streams".into(),
            "-show_format".into(),
            "-of".into(),
            "json".into(),
            path.as_os_str().into(),
        ];
        let data = self
            .run(&self.ffprobe, &args, Duration::from_secs(30))
            .await?;
        let p: Probe = serde_json::from_slice(&data)?;
        if p.streams.len() != 1
            || p.streams[0].codec_type.as_deref() != Some("audio")
            || !matches!(p.streams[0].channels, Some(1 | 2))
        {
            return Err(AppError::input(
                "Select audio-only media with one stream and at most two channels.",
            ));
        }
        let format = p.format.format_name.unwrap_or_default();
        if !format
            .split(',')
            .any(|f| ["wav", "mp3", "mov", "m4a", "aac", "flac"].contains(&f))
        {
            return Err(AppError::input(
                "Supported audio formats: WAV, MP3, M4A/AAC and FLAC.",
            ));
        }
        let duration = p
            .format
            .duration
            .and_then(|s| s.parse::<f64>().ok())
            .filter(|n| n.is_finite() && *n > 0.)
            .ok_or_else(|| AppError::input("Audio duration could not be measured."))?;
        if duration > MAX_SECONDS + 0.001 {
            return Err(AppError::input(
                "Original audio exceeds the 60 minute limit.",
            ));
        }
        Ok(AudioInfo { duration, bytes })
    }
    async fn prepare(&self, path: &Path, dir: &Path, s: &Snapshot) -> Result<PreparedAudio> {
        tokio::fs::create_dir_all(dir).await?;
        let pcm = dir.join("decoded.wav");
        let derivative = s.compression || s.silence_removal;
        let output = if derivative {
            dir.join(if s.compression {
                "upload.m4a"
            } else {
                "upload.wav"
            })
        } else {
            pcm.clone()
        };
        let mut args: Vec<std::ffi::OsString> = [
            "-nostdin",
            "-v",
            "error",
            "-y",
            "-protocol_whitelist",
            "file,pipe",
            "-i",
        ]
        .iter()
        .map(Into::into)
        .collect();
        args.push(path.as_os_str().into());
        if derivative {
            // One decoder feeds both outputs. Keep validation untrimmed and
            // convert the upload to the same PCM format before filtering/encoding.
            // Use two output mappings rather than requiring extra bundled filters.
            args.extend(
                [
                    "-map",
                    "0:a:0",
                    "-vn",
                    "-ac",
                    "1",
                    "-ar",
                    "24000",
                    "-c:a",
                    "pcm_s16le",
                ]
                .iter()
                .map(Into::into),
            );
            args.push(pcm.as_os_str().into());
            let mut filter =
                "aformat=sample_fmts=s16:sample_rates=24000:channel_layouts=mono".to_owned();
            if s.silence_removal {
                filter.push_str(",silenceremove=stop_periods=-1:stop_duration=1.5:stop_threshold=-60dB:stop_silence=0.25:detection=rms");
            }
            args.extend(["-map".into(), "0:a:0".into(), "-af".into(), filter.into()]);
            if s.compression {
                args.extend(
                    [
                        "-c:a",
                        "aac",
                        "-profile:a",
                        "aac_low",
                        "-b:a",
                        "48k",
                        "-movflags",
                        "+faststart",
                    ]
                    .iter()
                    .map(Into::into),
                );
            } else {
                args.extend(["-c:a", "pcm_s16le"].iter().map(Into::into));
            }
            args.push(output.as_os_str().into());
        } else {
            args.extend(
                [
                    "-map",
                    "0:a:0",
                    "-vn",
                    "-ac",
                    "1",
                    "-ar",
                    "24000",
                    "-c:a",
                    "pcm_s16le",
                ]
                .iter()
                .map(Into::into),
            );
            args.push(pcm.as_os_str().into());
        }
        self.run(&self.ffmpeg, &args, Duration::from_secs(600))
            .await?;
        let check = pcm.clone();
        tokio::task::spawn_blocking(move || validate_pcm(&check))
            .await
            .map_err(|_| AppError::new("media", "worker", "Audio validation stopped."))??;
        let bytes = tokio::fs::metadata(&output).await?.len();
        if derivative && bytes < 128 {
            return Err(AppError::new(
                "preparing",
                "no_speech",
                "No speech detected. Original audio is retained.",
            ));
        }
        if s.compression && bytes > 24_000_000 {
            return Err(AppError::new(
                "preparing",
                "size",
                "Compressed upload exceeds the 24 MB safety ceiling. Original audio is retained.",
            ));
        }
        Ok(PreparedAudio {
            path: output,
            bytes,
            mime: if s.compression {
                "audio/mp4"
            } else {
                "audio/wav"
            },
        })
    }
}
pub fn validate_pcm(path: &Path) -> Result<()> {
    let reader =
        hound::WavReader::open(path).map_err(|_| AppError::input("Decoded audio is invalid."))?;
    let spec = reader.spec();
    let duration = reader.duration() as f64 / spec.sample_rate as f64;
    if duration > MAX_SECONDS + 0.001 {
        return Err(AppError::input("Decoded original exceeds 60 minutes."));
    }
    let mut peak = 0u16;
    for sample in reader.into_samples::<i16>() {
        let s = sample.map_err(|_| AppError::input("Decoded audio is corrupt."))?;
        peak = peak.max(s.unsigned_abs());
    }
    if peak < 2 {
        return Err(AppError::new(
            "preparing",
            "no_speech",
            "No speech detected. Original audio is retained.",
        ));
    }
    Ok(())
}
#[cfg(windows)]
pub struct ChildJob(isize);
#[cfg(windows)]
impl ChildJob {
    pub fn attach(child: &tokio::process::Child) -> Result<Self> {
        use windows_sys::Win32::System::JobObjects::*;
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return Err(AppError::new(
                    "media",
                    "lifecycle",
                    "Could not supervise media process.",
                ));
            }
            let guard = Self(job as isize);
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of_val(&info) as u32,
            ) == 0
                || AssignProcessToJobObject(
                    job,
                    child
                        .raw_handle()
                        .ok_or_else(|| AppError::input("Media process handle unavailable."))?
                        as _,
                ) == 0
            {
                return Err(AppError::new(
                    "media",
                    "lifecycle",
                    "Could not supervise media process.",
                ));
            }
            Ok(guard)
        }
    }
}
#[cfg(windows)]
impl Drop for ChildJob {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0 as _);
        }
    }
}
