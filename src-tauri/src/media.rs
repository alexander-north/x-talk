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
    /// Validate a user-selected import, including the import size limit.
    async fn probe(&self, path: &Path) -> Result<AudioInfo>;
    /// Validate audio already retained by x-talk, such as a long native-rate recording.
    /// Format, stream and duration rules still apply; the import size limit does not.
    async fn inspect(&self, path: &Path) -> Result<AudioInfo> {
        self.probe(path).await
    }
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
        self.inspect(path).await
    }
    async fn inspect(&self, path: &Path) -> Result<AudioInfo> {
        let bytes = tokio::fs::metadata(path).await?.len();
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
/// Platform name of a bundled media tool: `ffmpeg.exe` on Windows, `ffmpeg` elsewhere.
pub fn tool_file_name(tool: &str) -> String {
    format!("{tool}{}", std::env::consts::EXE_SUFFIX)
}
/// Directories searched for system media tools in non-Windows development builds, after PATH.
/// Covers Homebrew on Apple Silicon and Intel, and Linux distribution packages.
pub const DEVELOPMENT_TOOL_DIRS: [&str; 3] = ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"];
/// Resolve one media tool. The bundled `<resources>/binaries/<tool>` always wins.
/// Only when `fallback` is set (debug builds on non-Windows) and the bundled tool is missing,
/// the first executable `<tool>` in `search` is used, so `brew install ffmpeg` is enough for
/// development. Without a match the bundled path is returned, and spawning it reports the
/// "Reinstall x-talk" error.
pub fn resolve_tool(tool: &str, bundled: &Path, search: &[PathBuf], fallback: bool) -> PathBuf {
    let name = tool_file_name(tool);
    let primary = bundled.join(&name);
    if !fallback || executable(&primary) {
        return primary;
    }
    search
        .iter()
        .map(|dir| dir.join(&name))
        .find(|candidate| executable(candidate))
        .unwrap_or(primary)
}
/// PATH entries followed by [`DEVELOPMENT_TOOL_DIRS`], for [`resolve_tool`].
pub fn development_search_path() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default();
    dirs.extend(DEVELOPMENT_TOOL_DIRS.iter().map(PathBuf::from));
    dirs
}
fn executable(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // Zero-byte placeholders satisfy the Tauri build script but are not tools.
        meta.is_file() && meta.len() > 0 && meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        meta.is_file() && meta.len() > 0
    }
}

/// Unix counterpart of the Windows job object: a registry of live media children.
/// Children stay in the app's process group, so signals sent to the group (terminal Ctrl+C,
/// `kill -- -<pgid>`) reach them too. `kill_on_drop` covers cancellation and dropped
/// futures; [`terminate_children`] runs on orderly app exit (Quit, Cmd+Q, logout, update
/// restart) to SIGKILL any decoder still registered. A SIGKILL of the app itself cannot be
/// intercepted on macOS; such a child ends at its next write to a closed pipe or when its
/// bounded job finishes.
#[cfg(unix)]
pub struct ChildJob(u32);
#[cfg(unix)]
fn live_children() -> &'static std::sync::Mutex<std::collections::HashSet<u32>> {
    static LIVE: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<u32>>> =
        std::sync::OnceLock::new();
    LIVE.get_or_init(Default::default)
}
#[cfg(unix)]
impl ChildJob {
    pub fn attach(child: &tokio::process::Child) -> Result<Self> {
        let pid = child.id().ok_or_else(|| {
            AppError::new("media", "lifecycle", "Could not supervise media process.")
        })?;
        live_children()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(pid);
        Ok(Self(pid))
    }
}
#[cfg(unix)]
impl Drop for ChildJob {
    fn drop(&mut self) {
        live_children()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.0);
    }
}
/// Kill every supervised media child. Call once the app is exiting.
#[cfg(unix)]
pub fn terminate_children() {
    terminate_where(|_| true);
}
#[cfg(unix)]
fn terminate_where(filter: impl Fn(u32) -> bool) {
    let pids: Vec<u32> = {
        let mut live = live_children().lock().unwrap_or_else(|e| e.into_inner());
        let pids: Vec<u32> = live.iter().copied().filter(|pid| filter(*pid)).collect();
        for pid in &pids {
            live.remove(pid);
        }
        pids
    };
    for pid in pids {
        if let Ok(pid) = libc::pid_t::try_from(pid) {
            // SAFETY: kill(2) has no memory-safety preconditions. A registered pid is a child
            // of this process that its guard has not released; guards are dropped right after
            // the child is reaped, so the pid cannot have been reused in between.
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
        }
    }
}
/// Windows children die with the job object when the process exits.
#[cfg(windows)]
pub fn terminate_children() {}
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

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    fn make_executable(path: &Path) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(path, b"#!/bin/sh\n").unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    #[test]
    fn tool_names_use_the_platform_executable_suffix() {
        let expected = if cfg!(windows) {
            "ffprobe.exe"
        } else {
            "ffprobe"
        };
        assert_eq!(tool_file_name("ffprobe"), expected);
    }
    #[test]
    fn release_resolution_always_uses_the_bundled_tool() {
        let bundled = Path::new("/nonexistent/binaries");
        assert_eq!(
            resolve_tool("ffmpeg", bundled, &[PathBuf::from("/usr/bin")], false),
            bundled.join(tool_file_name("ffmpeg"))
        );
    }
    #[cfg(unix)]
    #[test]
    fn development_resolution_prefers_bundled_then_searches_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let bundled = dir.path().join("binaries");
        let first = dir.path().join("first");
        let second = dir.path().join("second");
        for d in [&bundled, &first, &second] {
            std::fs::create_dir_all(d).unwrap();
        }
        let search = [first.clone(), second.clone()];
        // Nothing executable anywhere: keep the bundled path for the "reinstall" error.
        assert_eq!(
            resolve_tool("ffmpeg", &bundled, &search, true),
            bundled.join("ffmpeg")
        );
        // Zero-byte placeholders and non-executable files are skipped.
        std::fs::write(bundled.join("ffmpeg"), b"").unwrap();
        std::fs::write(first.join("ffmpeg"), b"data").unwrap();
        make_executable(&second.join("ffmpeg"));
        assert_eq!(
            resolve_tool("ffmpeg", &bundled, &search, true),
            second.join("ffmpeg")
        );
        make_executable(&first.join("ffmpeg"));
        assert_eq!(
            resolve_tool("ffmpeg", &bundled, &search, true),
            first.join("ffmpeg")
        );
        make_executable(&bundled.join("ffmpeg"));
        assert_eq!(
            resolve_tool("ffmpeg", &bundled, &search, true),
            bundled.join("ffmpeg")
        );
    }
    #[test]
    fn development_search_path_ends_with_homebrew_and_system_dirs() {
        let dirs = development_search_path();
        let expected: Vec<PathBuf> = DEVELOPMENT_TOOL_DIRS.iter().map(PathBuf::from).collect();
        assert!(dirs.ends_with(&expected));
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn exit_terminates_registered_children_and_released_guards_are_forgotten() {
        let spawn = || {
            Command::new("sleep")
                .arg("30")
                .kill_on_drop(true)
                .spawn()
                .unwrap()
        };
        let mut released = spawn();
        let guard = ChildJob::attach(&released).unwrap();
        let pid = released.id().unwrap();
        drop(guard);
        assert!(!live_children().lock().unwrap().contains(&pid));
        let mut child = spawn();
        let _guard = ChildJob::attach(&child).unwrap();
        let target = child.id().unwrap();
        // Scoped to this child: other tests may supervise media tools concurrently.
        terminate_where(|pid| pid == target);
        assert!(!live_children().lock().unwrap().contains(&target));
        let status = tokio::time::timeout(Duration::from_secs(5), child.wait())
            .await
            .expect("registered child must exit")
            .unwrap();
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(status.signal(), Some(libc::SIGKILL));
        // A released child is never signalled.
        assert!(
            tokio::time::timeout(Duration::from_millis(200), released.wait())
                .await
                .is_err()
        );
        released.kill().await.unwrap();
    }
}
