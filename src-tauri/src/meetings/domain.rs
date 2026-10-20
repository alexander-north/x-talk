use crate::domain::{id, now, AppError, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_SECONDS: f64 = 4.0 * 3600.0;
pub const BRIEF_MODEL: &str = "gpt-6-luna";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Audio,
    Video,
    Recording,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Word {
    pub text: String,
    pub start: Option<f64>,
    pub end: Option<f64>,
    pub confidence: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Turn {
    pub id: String,
    pub speaker: String,
    pub start: f64,
    pub end: f64,
    pub text: String,
    /// Immutable provider evidence; never realigned to corrected text.
    pub words: Vec<Word>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Correction {
    pub text: Option<String>,
    pub speaker: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SpeakerMerge {
    pub from: String,
    pub into: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Transcript {
    pub generation: String,
    pub provider: String,
    pub model: Option<String>,
    pub turns: Vec<Turn>,
    pub names: BTreeMap<String, String>,
    pub corrections: BTreeMap<String, Correction>,
    pub merges: Vec<SpeakerMerge>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub text: String,
    pub segment_ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionItem {
    pub text: String,
    pub segment_ids: Vec<String>,
    pub owner: Option<String>,
    pub due: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BriefContent {
    pub title: String,
    pub summary: String,
    pub topics: Vec<Evidence>,
    pub decisions: Vec<Evidence>,
    pub actions: Vec<ActionItem>,
    pub open_questions: Vec<Evidence>,
    pub suggested_follow_ups: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    #[default]
    Queued,
    Preparing,
    WaitingForKeys,
    Submitting,
    Polling,
    Briefing,
    Complete,
    Failed,
    SubmissionUnknown,
    Recovered,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Brief {
    pub status: Stage,
    pub provider: String,
    pub model: String,
    #[serde(default)]
    pub actual_model: Option<String>,
    pub input_generation: Option<String>,
    pub input_revision: Option<u64>,
    pub content: Option<BriefContent>,
    pub error: Option<String>,
}
impl Default for Brief {
    fn default() -> Self {
        Self {
            status: Stage::Queued,
            provider: "openai".into(),
            model: BRIEF_MODEL.into(),
            actual_model: None,
            input_generation: None,
            input_revision: None,
            content: None,
            error: None,
        }
    }
}

/// All managed paths are relative to the meeting's own audio directory.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Audio {
    pub download: String,
    pub prepared: Option<String>,
    pub assets: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Attempt {
    pub id: String,
    pub stage: Stage,
    pub remote_id: Option<String>,
    pub error: Option<String>,
    pub replacement: bool,
}
impl Attempt {
    pub fn queued(replacement: bool) -> Self {
        Self {
            id: id(),
            stage: Stage::Queued,
            remote_id: None,
            error: None,
            replacement,
        }
    }
    pub fn protects_audio(&self) -> bool {
        !matches!(self.stage, Stage::Complete)
    }
    pub fn recovery(&mut self) {
        self.stage = match self.stage {
            Stage::Submitting => Stage::SubmissionUnknown,
            Stage::Preparing => Stage::Queued,
            // A brief POST may have reached the provider before the process died.
            Stage::Briefing => Stage::SubmissionUnknown,
            _ => self.stage.clone(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Meeting {
    pub id: String,
    pub title: String,
    pub manual_title: bool,
    pub added_at: String,
    pub call_at: String,
    pub duration: Option<f64>,
    pub source: SourceKind,
    pub filename: Option<String>,
    pub revision: u64,
    pub content_revision: u64,
    pub transcript: Option<Transcript>,
    pub brief: Brief,
    pub audio: Option<Audio>,
    #[serde(default)]
    pub audio_hash: Option<String>,
    #[serde(default)]
    pub capture_warning: Option<String>,
    pub attempt: Attempt,
}
impl Meeting {
    pub fn new(title: String, source: SourceKind, filename: Option<String>) -> Self {
        let added_at = now();
        Self {
            id: id(),
            title,
            manual_title: false,
            call_at: added_at.clone(),
            added_at,
            duration: None,
            source,
            filename,
            revision: 0,
            content_revision: 0,
            transcript: None,
            brief: Brief::default(),
            audio: None,
            audio_hash: None,
            capture_warning: None,
            attempt: Attempt::queued(false),
        }
    }
    pub fn brief_stale(&self) -> bool {
        self.brief.content.is_some()
            && (self.brief.input_revision != Some(self.content_revision)
                || self.brief.input_generation.as_deref()
                    != self.transcript.as_ref().map(|t| t.generation.as_str()))
    }
    fn editable(&mut self) -> Result<&mut Transcript> {
        if self.attempt.replacement
            && !matches!(self.attempt.stage, Stage::Failed | Stage::Complete)
        {
            return Err(AppError::input(
                "Wait for re-transcription before editing the transcript.",
            ));
        }
        self.transcript
            .as_mut()
            .ok_or_else(|| AppError::input("This meeting has no transcript yet."))
    }
    pub fn retry(&mut self, transcription: bool) -> Result<()> {
        if !matches!(
            self.attempt.stage,
            Stage::Complete
                | Stage::Failed
                | Stage::SubmissionUnknown
                | Stage::Recovered
                | Stage::WaitingForKeys
        ) {
            return Err(AppError::input(
                "Wait for the current meeting job to finish.",
            ));
        }
        if transcription && self.audio.is_none() {
            return Err(AppError::input("Meeting audio is no longer retained."));
        }
        if !transcription && self.transcript.is_none() {
            return Err(AppError::input("Transcribe the meeting first."));
        }
        // A failed brief must not turn explicit re-transcription into a replay
        // of the old completed transcript. Only unfinished known jobs resume.
        let remote = if transcription
            && (self.transcript.is_none() || self.attempt.replacement)
            && self.attempt.stage != Stage::Complete
        {
            self.attempt.remote_id.clone()
        } else {
            None
        };
        self.attempt = Attempt::queued(transcription && self.transcript.is_some());
        if let Some(remote) = remote {
            self.attempt.remote_id = Some(remote);
            self.attempt.stage = Stage::Polling;
        }
        if !transcription {
            self.brief.status = Stage::Queued;
            self.brief.error = None;
        }
        Ok(())
    }
    pub fn correct(&mut self, turn: &str, correction: Option<Correction>) -> Result<()> {
        let t = self.editable()?;
        if !t.turns.iter().any(|s| s.id == turn) {
            return Err(AppError::input("Unknown transcript turn."));
        }
        if let Some(c) = correction {
            if c.text.as_ref().is_some_and(|s| s.len() > 100_000)
                || c.speaker
                    .as_ref()
                    .is_some_and(|s| !t.speaker_ids().contains(s))
            {
                return Err(AppError::input("Invalid transcript correction."));
            }
            t.corrections.insert(turn.into(), c);
        } else {
            t.corrections.remove(turn);
        }
        self.content_revision += 1;
        Ok(())
    }
    pub fn name_speaker(&mut self, speaker: &str, name: &str) -> Result<()> {
        let t = self.editable()?;
        if !t.speaker_ids().contains(speaker) || name.trim().is_empty() || name.len() > 200 {
            return Err(AppError::input(
                "Enter a speaker name of at most 200 characters.",
            ));
        }
        t.names.insert(speaker.into(), name.trim().into());
        self.content_revision += 1;
        Ok(())
    }
    pub fn merge(&mut self, from: &str, into: &str) -> Result<()> {
        let t = self.editable()?;
        if !t.speaker_ids().contains(from) || !t.speaker_ids().contains(into) {
            return Err(AppError::input("Unknown speaker."));
        }
        let from = t.resolve(from)?;
        let into = t.resolve(into)?;
        if from == into {
            return Err(AppError::input("These speakers are already merged."));
        }
        t.merges.push(SpeakerMerge { from, into });
        self.content_revision += 1;
        Ok(())
    }
    pub fn undo_merge(&mut self) -> Result<()> {
        if self.editable()?.merges.pop().is_none() {
            return Err(AppError::input("There is no speaker merge to undo."));
        }
        self.content_revision += 1;
        Ok(())
    }
}
impl Transcript {
    pub fn speaker_ids(&self) -> BTreeSet<String> {
        self.turns.iter().map(|t| t.speaker.clone()).collect()
    }
    pub fn resolve(&self, speaker: &str) -> Result<String> {
        let mut current = speaker.to_string();
        let mut visited = BTreeSet::new();
        loop {
            if !visited.insert(current.clone()) {
                return Err(AppError::input("Speaker merge contains a cycle."));
            }
            match self.merges.iter().find(|m| m.from == current) {
                Some(m) => current = m.into.clone(),
                None => return Ok(current),
            }
        }
    }
    pub fn validate(&self) -> Result<()> {
        let mut ids = BTreeSet::new();
        for turn in &self.turns {
            if turn.id.is_empty()
                || !ids.insert(&turn.id)
                || turn.speaker.is_empty()
                || !turn.start.is_finite()
                || !turn.end.is_finite()
                || turn.start < 0.0
                || turn.end < turn.start
                || turn.end > MAX_SECONDS + 1.0
            {
                return Err(AppError::input(
                    "Transcript contains invalid turns or timestamps.",
                ));
            }
            self.resolve(&turn.speaker)?;
        }
        Ok(())
    }
}
impl BriefContent {
    pub fn validate(&self, transcript: &Transcript) -> Result<()> {
        let ids: BTreeSet<_> = transcript.turns.iter().map(|t| t.id.as_str()).collect();
        if self.title.trim().is_empty() || self.title.len() > 720 || self.summary.is_empty() {
            return Err(AppError::input(
                "The meeting brief has an invalid title or summary.",
            ));
        }
        for (text, refs) in self
            .topics
            .iter()
            .chain(&self.decisions)
            .chain(&self.open_questions)
            .map(|e| (&e.text, &e.segment_ids))
            .chain(self.actions.iter().map(|e| (&e.text, &e.segment_ids)))
        {
            if text.trim().is_empty()
                || refs.is_empty()
                || refs.len() > 3
                || refs.iter().any(|id| !ids.contains(id.as_str()))
            {
                return Err(AppError::input(
                    "The brief cites invalid transcript evidence.",
                ));
            }
        }
        Ok(())
    }
}
