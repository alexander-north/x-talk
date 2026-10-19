//! Bounded, interruptible event-cue playback. Capture owns priority over feedback.
use crate::{domain::*, storage::Store};
use std::{
    collections::VecDeque,
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone)]
struct Cue {
    event: String,
    preview: bool,
    queued: Instant,
}
#[derive(Default)]
struct Schedule {
    epoch: u64,
    superseded_through: i64,
    recording: bool,
    closed: bool,
    pending: VecDeque<Cue>,
    until: Option<Instant>,
}
impl Schedule {
    fn context(&self) -> Option<u64> {
        (!self.recording && !self.closed).then_some(self.epoch)
    }
    fn job_context(&self, seq: i64) -> Option<u64> {
        if seq <= self.superseded_through {
            None
        } else {
            self.context()
        }
    }
    fn interrupt(&mut self) {
        self.pending.clear();
        self.until = None;
    }
    fn begin_capture(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        self.recording = true;
        self.interrupt();
    }
    fn enqueue(&mut self, event: &str, preview: bool, context: Option<u64>, now: Instant) -> bool {
        if self.closed {
            return false;
        }
        if preview {
            if self.recording {
                return false;
            }
            self.interrupt();
        } else if matches!(event, "started" | "stopped" | "canceled") {
            self.recording = event == "started";
            self.interrupt();
        } else if self.recording || context != Some(self.epoch) {
            return false;
        }
        // Prefer dropping excess feedback over playing a growing backlog.
        if self.pending.len() >= 4 {
            return false;
        }
        self.pending.push_back(Cue {
            event: event.into(),
            preview,
            queued: now,
        });
        true
    }
    fn next(&mut self, now: Instant) -> Option<Cue> {
        if self.closed || self.until.is_some_and(|end| now < end) {
            return None;
        }
        self.until = None;
        while let Some(cue) = self.pending.pop_front() {
            if now.duration_since(cue.queued) <= Duration::from_millis(1500) {
                return Some(cue);
            }
        }
        None
    }
}

pub struct CuePlayer {
    store: Store,
    shared: Arc<(Mutex<Schedule>, Condvar)>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl CuePlayer {
    pub fn new(store: Store) -> Result<Self> {
        let shared = Arc::new((Mutex::new(Schedule::default()), Condvar::new()));
        let thread_shared = shared.clone();
        let thread_store = store.clone();
        let worker = std::thread::Builder::new()
            .name("event-cues".into())
            .spawn(move || {
                let (lock, wake) = &*thread_shared;
                let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
                loop {
                    if state.closed {
                        break;
                    }
                    let now = Instant::now();
                    if let Some(cue) = state.next(now) {
                        // Recheck at playback, so a disabled queued cue stays silent.
                        let enabled = thread_store
                            .settings()
                            .is_ok_and(|s| enabled(&s.sounds, &cue.event));
                        if cue.preview || enabled {
                            if play(&cue.event) {
                                tracing::info!(
                                    event = cue.event,
                                    preview = cue.preview,
                                    sound_set = crate::sounds::VERSION,
                                    "cue_started"
                                );
                                state.until = Some(
                                    Instant::now()
                                        + duration(&cue.event)
                                        + Duration::from_millis(20),
                                );
                            } else {
                                tracing::warn!(event = cue.event, "cue_playback_failed");
                            }
                        }
                        continue;
                    }
                    state = if let Some(end) = state.until {
                        wake.wait_timeout(state, end.saturating_duration_since(Instant::now()))
                            .unwrap_or_else(|e| e.into_inner())
                            .0
                    } else {
                        wake.wait(state).unwrap_or_else(|e| e.into_inner())
                    };
                }
            })?;
        Ok(Self {
            store,
            shared,
            worker: Some(worker),
        })
    }
    pub fn context(&self) -> Option<u64> {
        self.shared
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .context()
    }
    pub fn job_context(&self, seq: i64) -> Option<u64> {
        let state = self.shared.0.lock().unwrap_or_else(|e| e.into_inner());
        state.job_context(seq)
    }
    /// Stop app cues before opening capture, without dropping any microphone samples.
    pub fn begin_capture(&self) {
        let cutoff = self.store.last_sequence().unwrap_or(i64::MAX);
        let mut state = self.shared.0.lock().unwrap_or_else(|e| e.into_inner());
        state.begin_capture();
        state.superseded_through = cutoff;
        stop();
        self.shared.1.notify_one();
    }
    pub fn idle(&self) {
        let mut state = self.shared.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.recording {
            state.recording = false;
            state.interrupt();
            stop();
            self.shared.1.notify_one();
        }
    }
    pub fn submit(&self, event: &str, preview: bool, context: Option<u64>) -> bool {
        let mut state = self.shared.0.lock().unwrap_or_else(|e| e.into_inner());
        let priority = preview || matches!(event, "started" | "stopped" | "canceled");
        let accepted = state.enqueue(event, preview, context, Instant::now());
        if accepted
            && !preview
            && !self
                .store
                .settings()
                .is_ok_and(|s| enabled(&s.sounds, event))
        {
            state.pending.pop_back();
        }
        if accepted {
            if priority {
                stop();
            }
            self.shared.1.notify_one();
        } else {
            tracing::info!(event, preview, "cue_suppressed");
        }
        accepted
    }
    pub fn shutdown(&self) {
        let mut state = self.shared.0.lock().unwrap_or_else(|e| e.into_inner());
        state.closed = true;
        state.interrupt();
        stop();
        self.shared.1.notify_one();
    }
}
impl Drop for CuePlayer {
    fn drop(&mut self) {
        self.shutdown();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
fn enabled(s: &Sounds, event: &str) -> bool {
    match event {
        "started" => s.started,
        "stopped" => s.stopped,
        "canceled" => s.canceled,
        "transcribed" => s.transcribed,
        "transformed" => s.transformed,
        _ => false,
    }
}
fn duration(event: &str) -> Duration {
    Duration::from_millis(match event {
        "started" => 400,
        "stopped" => 330,
        "canceled" => 320,
        "transcribed" => 440,
        "transformed" => 350,
        _ => 0,
    })
}
#[cfg(windows)]
fn play(event: &str) -> bool {
    let Ok(bytes) = crate::sounds::asset(event) else {
        return false;
    };
    // Embedded bytes have static lifetime, including throughout asynchronous playback.
    unsafe {
        windows_sys::Win32::Media::Audio::PlaySoundW(
            bytes.as_ptr().cast(),
            std::ptr::null_mut(),
            windows_sys::Win32::Media::Audio::SND_MEMORY
                | windows_sys::Win32::Media::Audio::SND_ASYNC
                | windows_sys::Win32::Media::Audio::SND_NODEFAULT,
        ) != 0
    }
}
#[cfg(windows)]
fn stop() {
    unsafe {
        windows_sys::Win32::Media::Audio::PlaySoundW(std::ptr::null(), std::ptr::null_mut(), 0);
    }
}
#[cfg(not(windows))]
fn play(_: &str) -> bool {
    false
}
#[cfg(not(windows))]
fn stop() {}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fast_completions_wait_for_the_whole_previous_phrase() {
        let mut s = Schedule::default();
        let t = Instant::now();
        assert!(s.enqueue("transcribed", false, Some(0), t));
        assert_eq!(s.next(t).unwrap().event, "transcribed");
        s.until = Some(t + duration("transcribed"));
        assert!(s.enqueue("transformed", false, Some(0), t));
        assert!(s.next(t + Duration::from_millis(439)).is_none());
        assert_eq!(
            s.next(t + Duration::from_millis(440)).unwrap().event,
            "transformed"
        );
    }
    #[test]
    fn capture_interrupts_and_old_jobs_stay_silent_after_capture_ends() {
        let mut s = Schedule::default();
        let t = Instant::now();
        let old = s.context();
        s.enqueue("transcribed", false, old, t);
        s.begin_capture();
        assert!(s.pending.is_empty());
        assert!(s.context().is_none());
        assert!(!s.enqueue("transformed", false, old, t));
        assert!(!s.enqueue("transformed", true, None, t));
        assert!(s.enqueue("started", false, None, t));
        assert!(s.enqueue("canceled", false, None, t));
        assert!(!s.enqueue("transformed", false, old, t));
        assert!(!s.enqueue("transformed", false, None, t));
        assert_eq!(s.next(t).unwrap().event, "canceled");
        assert!(s.enqueue("transformed", false, s.context(), t));
    }
    #[test]
    fn preview_replaces_preview_and_backlogs_expire_and_are_bounded() {
        let mut s = Schedule::default();
        let t = Instant::now();
        s.enqueue("started", true, None, t);
        s.enqueue("stopped", true, None, t);
        assert_eq!(s.pending.len(), 1);
        assert_eq!(s.next(t).unwrap().event, "stopped");
        for _ in 0..4 {
            assert!(s.enqueue("transcribed", false, Some(0), t));
        }
        assert!(!s.enqueue("transcribed", false, Some(0), t));
        assert!(s.next(t + Duration::from_secs(2)).is_none());
        s.closed = true;
        assert!(!s.enqueue("started", false, None, t));
    }
    #[test]
    fn queued_old_jobs_are_silent_but_fresh_jobs_and_retries_can_notify() {
        let mut s = Schedule::default();
        s.begin_capture();
        s.superseded_through = 7;
        assert!(s.job_context(8).is_none());
        s.recording = false;
        assert!(s.job_context(7).is_none());
        assert_eq!(s.job_context(8), Some(s.epoch));
    }
    #[test]
    fn timing_matches_embedded_audio() {
        for event in [
            "started",
            "stopped",
            "canceled",
            "transcribed",
            "transformed",
        ] {
            let r =
                hound::WavReader::new(std::io::Cursor::new(crate::sounds::asset(event).unwrap()))
                    .unwrap();
            assert_eq!(
                u64::from(r.duration()) * 1000 / u64::from(r.spec().sample_rate),
                duration(event).as_millis() as u64
            );
        }
    }
}
