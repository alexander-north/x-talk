use super::domain::*;
use crate::domain::Result;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Serialize)]
pub struct ExportTurn {
    pub id: String,
    pub speaker_id: String,
    pub start: f64,
    pub end: f64,
    pub text: String,
    pub original_text: String,
    pub original_speaker_id: String,
    pub text_changed: bool,
    pub speaker_changed: bool,
    pub original_words: Vec<Word>,
    pub correction: Correction,
}
#[derive(Serialize)]
pub struct ExportSpeaker {
    pub id: String,
    pub name: String,
    pub turn_count: usize,
    pub word_count: usize,
    pub estimated_turn_coverage_seconds: f64,
}
#[derive(Serialize)]
pub struct ExportTranscript {
    pub generation: String,
    pub provider: String,
    pub model: Option<String>,
    pub speakers: Vec<ExportSpeaker>,
    pub turns: Vec<ExportTurn>,
}
#[derive(Serialize)]
pub struct ExportBrief {
    #[serde(flatten)]
    pub brief: Brief,
    pub stale: bool,
}
#[derive(Serialize)]
pub struct MeetingExport {
    pub format: &'static str,
    pub schema_version: u32,
    pub id: String,
    pub title: String,
    pub added_at: String,
    pub call_at: String,
    pub duration_seconds: Option<f64>,
    pub source: SourceKind,
    pub filename: Option<String>,
    pub audio_available: bool,
    pub capture_warning: Option<String>,
    pub revision: u64,
    pub content_revision: u64,
    pub transcription_status: Stage,
    pub transcript: Option<ExportTranscript>,
    pub brief: ExportBrief,
    pub measurement_definitions: &'static str,
}
pub fn project(meeting: &Meeting) -> Result<MeetingExport> {
    let transcript = meeting
        .transcript
        .as_ref()
        .map(|t| -> Result<_> {
            t.validate()?;
            let mut speakers: BTreeMap<String, (ExportSpeaker, Vec<(f64, f64)>)> = BTreeMap::new();
            let mut turns = Vec::with_capacity(t.turns.len());
            for turn in &t.turns {
                let edit = t.corrections.get(&turn.id);
                let text = edit
                    .and_then(|e| e.text.as_ref())
                    .unwrap_or(&turn.text)
                    .clone();
                let speaker = t.resolve(
                    edit.and_then(|e| e.speaker.as_deref())
                        .unwrap_or(&turn.speaker),
                )?;
                let (stats, intervals) = speakers.entry(speaker.clone()).or_insert_with(|| {
                    (
                        ExportSpeaker {
                            id: speaker.clone(),
                            name: t
                                .names
                                .get(&speaker)
                                .cloned()
                                .unwrap_or_else(|| format!("Speaker {speaker}")),
                            turn_count: 0,
                            word_count: 0,
                            estimated_turn_coverage_seconds: 0.0,
                        },
                        Vec::new(),
                    )
                });
                stats.turn_count += 1;
                stats.word_count += text.split_whitespace().count();
                intervals.push((turn.start, turn.end));
                turns.push(ExportTurn {
                    id: turn.id.clone(),
                    speaker_id: speaker.clone(),
                    start: turn.start,
                    end: turn.end,
                    text_changed: text != turn.text,
                    speaker_changed: speaker != turn.speaker,
                    text,
                    original_text: turn.text.clone(),
                    original_speaker_id: turn.speaker.clone(),
                    original_words: turn.words.clone(),
                    correction: edit.cloned().unwrap_or_default(),
                });
            }
            let speakers = speakers
                .into_values()
                .map(|(mut stats, mut intervals)| {
                    intervals.sort_by(|a, b| a.0.total_cmp(&b.0));
                    let mut end: f64 = 0.0;
                    for (start, stop) in intervals {
                        stats.estimated_turn_coverage_seconds += (stop - start.max(end)).max(0.0);
                        end = end.max(stop);
                    }
                    stats
                })
                .collect();
            Ok(ExportTranscript {
                generation: t.generation.clone(),
                provider: t.provider.clone(),
                model: t.model.clone(),
                speakers,
                turns,
            })
        })
        .transpose()?;
    Ok(MeetingExport {
        format: "x-talk.meeting", schema_version: 1,
        id: meeting.id.clone(), title: meeting.title.clone(),
        added_at: meeting.added_at.clone(), call_at: meeting.call_at.clone(),
        duration_seconds: meeting.duration, source: meeting.source.clone(),
        filename: meeting.filename.clone(), audio_available: meeting.audio.is_some(),
        capture_warning: meeting.capture_warning.clone(),
        revision: meeting.revision, content_revision: meeting.content_revision,
        transcription_status: meeting.attempt.stage.clone(), transcript,
        brief: ExportBrief { brief: meeting.brief.clone(), stale: meeting.brief_stale() },
        measurement_definitions: "Speaker time is the union of diarized turn intervals, including pauses. Overlap between different speakers can make totals exceed meeting duration. Word counts use corrected text. Original word timing and confidence describe provider text, not corrected text.",
    })
}
pub fn json(meeting: &Meeting) -> Result<String> {
    Ok(serde_json::to_string_pretty(&project(meeting)?)?)
}
