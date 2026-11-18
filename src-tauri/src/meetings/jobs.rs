use super::{
    domain::*,
    providers::{MeetingProvider, Poll},
    store::MeetingStore,
};
use crate::domain::{AppError, Result};
use std::sync::Arc;

pub struct Worker {
    pub store: MeetingStore,
    pub provider: Arc<dyn MeetingProvider>,
}
impl Worker {
    /// One durable stage at a time. Known jobs are polled without another POST.
    pub async fn step(&self, meeting: Meeting) -> Result<bool> {
        let id = &meeting.id;
        let attempt = &meeting.attempt.id;
        match meeting.attempt.stage {
            Stage::Queued | Stage::WaitingForKeys => {
                let need_transcript = meeting.transcript.is_none() || meeting.attempt.replacement;
                let provider = if need_transcript {
                    "assemblyai"
                } else {
                    "openai"
                };
                if let Err(error) = self.provider.check_key(provider) {
                    if meeting.attempt.stage != Stage::WaitingForKeys {
                        self.store.publish(id, attempt, move |m| {
                            m.attempt.stage = Stage::WaitingForKeys;
                            m.attempt.error = Some(error.message);
                            Ok(())
                        })?;
                    }
                    return Ok(false);
                }
                if !need_transcript {
                    return self.generate_brief(meeting).await;
                }
                let _lease = self.store.lease(id)?;
                let audio = meeting
                    .audio
                    .as_ref()
                    .ok_or_else(|| AppError::input("Meeting audio is unavailable."))?;
                let path = self
                    .store
                    .audio_path(id, audio.prepared.as_deref().unwrap_or(&audio.download))?;
                self.store.publish(id, attempt, |m| {
                    m.attempt.stage = Stage::Preparing;
                    m.attempt.error = None;
                    Ok(())
                })?;
                let upload = match self.provider.upload(&path).await {
                    Ok(url) => url,
                    Err(error) => {
                        self.fail(&meeting, error, false)?;
                        return Ok(true);
                    }
                };
                // Durable marker before the billable POST. Never replay after an ambiguous exit.
                self.store.publish(id, attempt, |m| {
                    m.attempt.stage = Stage::Submitting;
                    Ok(())
                })?;
                match self.provider.submit(&upload).await {
                    Ok(remote) => {
                        self.store.publish(id, attempt, move |m| {
                            m.attempt.remote_id = Some(remote);
                            m.attempt.stage = Stage::Polling;
                            Ok(())
                        })?;
                    }
                    Err(error) => self.fail(&meeting, error, true)?,
                }
                Ok(true)
            }
            Stage::Polling => {
                let remote = meeting
                    .attempt
                    .remote_id
                    .as_deref()
                    .ok_or_else(|| AppError::input("Missing saved provider job ID."))?;
                match self.provider.poll(remote).await {
                    Ok(Poll::Pending) => Ok(false),
                    Ok(Poll::Complete(transcript)) => {
                        if let Err(error) = transcript.validate() {
                            self.fail(&meeting, error, false)?;
                            return Ok(true);
                        }
                        self.store.publish(id, attempt, move |m| {
                            m.transcript = Some(transcript);
                            m.content_revision += 1;
                            m.brief = Brief::default();
                            m.attempt.replacement = false;
                            m.attempt.stage = Stage::Queued;
                            m.attempt.error = None;
                            Ok(())
                        })?;
                        Ok(true)
                    }
                    Err(error)
                        if matches!(
                            error.category.as_str(),
                            "transport" | "timeout" | "rate_limit" | "provider_unavailable"
                        ) =>
                    {
                        // Safe GET retry; leave the remote ID and stage intact.
                        if meeting.attempt.error.as_deref() != Some(&error.message) {
                            self.store.publish(id, attempt, move |m| {
                                m.attempt.error = Some(error.message);
                                Ok(())
                            })?;
                        }
                        Ok(false)
                    }
                    Err(error) => {
                        self.fail(&meeting, error, false)?;
                        Ok(true)
                    }
                }
            }
            _ => Ok(false),
        }
    }
    async fn generate_brief(&self, meeting: Meeting) -> Result<bool> {
        let generation = meeting.transcript.as_ref().map(|t| t.generation.clone());
        let revision = meeting.content_revision;
        self.store
            .publish(&meeting.id, &meeting.attempt.id, move |m| {
                m.attempt.stage = Stage::Briefing;
                m.attempt.error = None;
                m.brief.status = Stage::Briefing;
                m.brief.error = None;
                Ok(())
            })?;
        let result = self.provider.brief(&meeting).await.and_then(|content| {
            content.content.validate(
                meeting
                    .transcript
                    .as_ref()
                    .ok_or_else(|| AppError::input("Missing transcript."))?,
            )?;
            Ok(content)
        });
        match result {
            Ok(generated) => {
                let content = generated.content;
                self.store
                    .publish(&meeting.id, &meeting.attempt.id, move |m| {
                        if !m.manual_title {
                            m.title = content.title.clone();
                        }
                        m.brief = Brief {
                            status: Stage::Complete,
                            provider: "openai".into(),
                            model: BRIEF_MODEL.into(),
                            actual_model: generated.model,
                            input_generation: generation,
                            input_revision: Some(revision),
                            content: Some(content),
                            error: None,
                        };
                        m.attempt.stage = Stage::Complete;
                        m.attempt.error = None;
                        Ok(())
                    })?;
                self.store.prune()?;
            }
            Err(error) => {
                let unknown = matches!(
                    error.category.as_str(),
                    "transport" | "timeout" | "provider_unavailable"
                );
                self.store
                    .publish(&meeting.id, &meeting.attempt.id, move |m| {
                        let stage = if unknown {
                            Stage::SubmissionUnknown
                        } else {
                            Stage::Failed
                        };
                        m.attempt.stage = stage.clone();
                        m.attempt.error = Some(error.message.clone());
                        m.brief.status = stage;
                        m.brief.error = Some(error.message);
                        Ok(())
                    })?;
            }
        }
        Ok(true)
    }
    fn fail(&self, m: &Meeting, error: AppError, unknown: bool) -> Result<()> {
        self.store.publish(&m.id, &m.attempt.id, move |m| {
            m.attempt.stage = if unknown {
                Stage::SubmissionUnknown
            } else {
                Stage::Failed
            };
            if error.category == "provider_failed" {
                // The provider explicitly ended this job. A deliberate retry
                // must submit a new job instead of polling the failed ID forever.
                m.attempt.remote_id = None;
            }
            m.attempt.error = Some(error.message);
            Ok(())
        })?;
        Ok(())
    }
}
