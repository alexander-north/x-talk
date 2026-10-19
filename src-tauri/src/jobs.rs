use crate::{
    domain::*,
    media::MediaProcessor,
    providers::{TranscriptionProvider, TransformationProvider},
    storage::Store,
};
use async_trait::async_trait;
use std::sync::Arc;
#[async_trait]
pub trait ClipboardWriter: Send + Sync {
    async fn copy(&self, text: &str) -> Result<()>;
}
pub trait Feedback: Send + Sync {
    fn changed(&self);
    fn sound(&self, event: &str);
    fn sound_context(&self, _job: &Job) -> Option<u64> {
        Some(0)
    }
    fn sound_in_context(&self, event: &str, _context: Option<u64>) {
        self.sound(event);
    }
}
pub struct Worker {
    pub store: Store,
    pub media: Arc<dyn MediaProcessor>,
    pub transcription: Arc<dyn TranscriptionProvider>,
    pub transformation: Arc<dyn TransformationProvider>,
    pub clipboard: Arc<dyn ClipboardWriter>,
    pub feedback: Arc<dyn Feedback>,
}
impl Worker {
    pub async fn drain(&self) -> Result<()> {
        while self.run_next().await? {}
        Ok(())
    }
    pub async fn drain_limit(&self, cutoff: Option<i64>) -> Result<()> {
        loop {
            if cutoff.is_some_and(|n| {
                self.store
                    .next_sequence()
                    .ok()
                    .flatten()
                    .is_none_or(|seq| seq > n)
            }) {
                break;
            }
            if !self.run_next().await? {
                break;
            }
        }
        Ok(())
    }
    pub async fn run_next(&self) -> Result<bool> {
        let Some(mut j) = self.store.next()? else {
            return Ok(false);
        };
        tracing::info!(
            job_id = j.id,
            entry_id = j.entry_id,
            stage = j.stage,
            "job_started"
        );
        self.feedback.changed();
        let started = std::time::Instant::now();
        let job_id = j.id.clone();
        let result = tokio::select! {
            result = self.process(&mut j) => result,
            result = async {
                loop {
                    if self.store.cancellation_requested(&job_id)? { return Ok::<(), AppError>(()); }
                    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
                }
            } => { result?; Err(AppError::new("processing", "canceled", "Processing canceled.")) }
        };
        if self.store.cancellation_requested(&j.id)? {
            // Drop the in-flight future before acknowledging cancellation and admitting new work.
            let _ = tokio::fs::remove_dir_all(self.store.root.join("tmp").join(&j.id)).await;
            self.store.finish_cancellation(&j.id)?;
            self.feedback.changed();
            return Ok(true);
        }
        if let Err(mut e) = result {
            e.stage = j.stage.clone();
            tracing::warn!(stage = j.stage, category = e.category, "stage_failed");
            j.status = if j.can_poll_transcription()
                && ["poll_timeout", "timeout", "transport", "rate_limit", "provider_unavailable"]
                    .contains(&e.category.as_str())
            {
                "paused"
            } else if j.requires_explicit_retry()
                && ["timeout", "transport", "provider_unavailable"].contains(&e.category.as_str())
            {
                e.message = format!("{} The request outcome may be unknown. Explicitly retry this stage; another charge may apply.", e.message);
                e.recovery = "Explicitly retry the affected stage; another charge may apply.".into();
                "unknown"
            } else {
                "failed"
            }.into();
            j.error = Some(e);
            if let Err(error) = self.store.checkpoint(&j) {
                if error.category != "canceled" {
                    return Err(error);
                }
                self.store.finish_cancellation(&j.id)?;
            }
        }
        tracing::info!(
            job_id = j.id,
            entry_id = j.entry_id,
            status = j.status,
            stage = j.stage,
            duration_ms = started.elapsed().as_millis() as u64,
            "job_finished"
        );
        let _ = tokio::fs::remove_dir_all(self.store.root.join("tmp").join(&j.id)).await;
        self.store.retention()?;
        self.feedback.changed();
        Ok(true)
    }
    async fn stage(&self, j: &mut Job, stage: &str) -> Result<()> {
        j.stage = stage.into();
        self.store.checkpoint(j)?;
        tracing::info!(
            job_id = j.id,
            entry_id = j.entry_id,
            stage,
            "job_stage_changed"
        );
        self.feedback.changed();
        Ok(())
    }
    async fn deliver(&self, j: &mut Job, text: &str, enabled: bool) -> Result<()> {
        if j.eligible && j.kind == "initial" && enabled {
            tracing::info!(job_id = j.id, "clipboard_delivery_started");
            j.delivery = "writing".into();
            self.store.checkpoint(j)?;
            j.delivery = match self.clipboard.copy(text).await {
                Ok(()) => "copied",
                Err(_) => "copy_failed",
            }
            .into();
        } else if j.delivery != "copied" && j.delivery != "copy_failed" {
            j.delivery = "manual_only".into();
        }
        self.store.checkpoint(j)?;
        Ok(())
    }
    pub async fn process(&self, j: &mut Job) -> Result<()> {
        let sound_context = self.feedback.sound_context(j);
        let initial_stage = j.stage.clone();
        if j.kind != "transform" && !["transforming", "ready"].contains(&initial_stage.as_str()) {
            let entry = self.store.entry(&j.entry_id)?;
            let relative = entry
                .audio
                .ok_or_else(|| AppError::input("Original audio is no longer retained."))?;
            self.stage(j, "preparing").await?;
            let (text, elapsed_ms) = if j.snapshot.transcription_mode == TranscriptionMode::Realtime
                && j.kind == "initial"
            {
                // Validate the retained original, but never prepare/upload it a second time.
                self.media.probe(&self.store.path(&relative)?).await?;
                self.stage(j, "transcribing").await?;
                let outcome = j.realtime.clone().ok_or_else(|| crate::realtime::failure("stream_interrupted", "Real-time transcription was interrupted. Retry the retained audio explicitly from Recordings."))?;
                let text = outcome.result?;
                if text.trim().is_empty() {
                    return Err(crate::realtime::failure(
                        "no_speech",
                        "No speech detected. Original audio is retained.",
                    ));
                }
                (text, outcome.finalize_ms)
            } else {
                let audio = self
                    .media
                    .prepare(
                        &self.store.path(&relative)?,
                        &self.store.root.join("tmp").join(&j.id),
                        &j.snapshot,
                    )
                    .await?;
                self.stage(j, "transcribing").await?;
                let checkpoint = j.clone();
                let store = self.store.clone();
                let remote_id = std::sync::Mutex::new(j.remote_id.clone());
                let persist = |remote: String| {
                    let mut copy = checkpoint.clone();
                    copy.remote_id = Some(remote.clone());
                    store.checkpoint(&copy)?;
                    *remote_id
                        .lock()
                        .map_err(|_| AppError::input("Remote job state is unavailable."))? =
                        Some(remote);
                    Ok(())
                };
                let stage_started = std::time::Instant::now();
                let result = self.transcription.transcribe(&audio, j, &persist).await;
                let elapsed_ms = stage_started.elapsed().as_millis() as u64;
                j.remote_id = remote_id
                    .into_inner()
                    .map_err(|_| AppError::input("Remote job state is unavailable."))?;
                let text = result?;
                (text, elapsed_ms)
            };
            j.stage = if j.kind == "initial" || j.kind == "import" {
                if j.snapshot.preset.is_some() {
                    "transforming"
                } else {
                    "ready"
                }
            } else {
                "ready"
            }
            .into();
            j.realtime = None;
            self.store.transcript(j, text.clone(), Some(elapsed_ms))?;
            self.deliver(j, &text, j.snapshot.copy_raw).await?;
            self.feedback.sound_in_context("transcribed", sound_context);
            self.feedback.changed();
        }
        if j.stage != "ready" && j.kind != "transcribe" && (j.snapshot.preset.is_some()) {
            if j.input.is_none() {
                let e = self.store.entry(&j.entry_id)?;
                j.input = e.transcript;
                j.input_revision = e.transcript_revision;
            }
            self.stage(j, "transforming").await?;
            let p = j
                .snapshot
                .preset
                .as_ref()
                .ok_or_else(|| AppError::input("No transformation preset selected."))?;
            let stage_started = std::time::Instant::now();
            let text = self
                .transformation
                .transform(
                    j.input
                        .as_deref()
                        .ok_or_else(|| AppError::input("No saved transcript."))?,
                    p,
                )
                .await?;
            j.stage = "ready".into();
            self.store.transformed(
                j,
                text.clone(),
                Some(stage_started.elapsed().as_millis() as u64),
            )?;
            self.deliver(j, &text, auto_copy(j)).await?;
            self.feedback.sound_in_context("transformed", sound_context);
            self.feedback.changed();
        }
        self.stage(j, "ready").await?;
        if initial_stage == "ready" {
            j.delivery = "manual_only".into();
        }
        tracing::info!(
            job_id = j.id,
            status = j.delivery,
            "clipboard_delivery_result"
        );
        j.stage = "complete".into();
        j.status = "complete".into();
        j.error = None;
        self.store.checkpoint(j)?;
        Ok(())
    }
}
