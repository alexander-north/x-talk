use serde::{Deserialize, Serialize};
use uuid::Uuid;
pub const CAPACITY: usize = 20;
pub const MAX_SECONDS: f64 = 3600.0;
pub type Result<T> = std::result::Result<T, AppError>;
#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error)]
#[error("{message}")]
pub struct AppError {
    pub stage: String,
    pub category: String,
    pub message: String,
    pub recovery: String,
}
impl AppError {
    pub fn new(stage: &str, category: &str, message: impl Into<String>) -> Self {
        Self {
            stage: stage.into(),
            category: category.into(),
            message: message.into(),
            recovery: "Review settings and retry the affected stage.".into(),
        }
    }
    pub fn input(message: impl Into<String>) -> Self {
        Self::new("validation", "invalid_input", message)
    }
}
impl From<rusqlite::Error> for AppError {
    fn from(error: rusqlite::Error) -> Self {
        if let rusqlite::Error::SqliteFailure(code, _) = &error {
            tracing::error!(kind = ?code.code, sqlite_code = code.extended_code, "database_operation_failed");
        } else {
            tracing::error!("database_operation_failed");
        }
        Self::new(
            "storage",
            "database",
            "Local database operation failed. Check available disk space.",
        )
    }
}
impl From<std::io::Error> for AppError {
    fn from(error: std::io::Error) -> Self {
        tracing::error!(kind = ?error.kind(), os_error = error.raw_os_error(), "file_operation_failed");
        Self::new(
            "storage",
            "io",
            "File operation failed. Check disk space and file access.",
        )
    }
}
impl From<serde_json::Error> for AppError {
    fn from(error: serde_json::Error) -> Self {
        tracing::warn!(line = error.line(), column = error.column(), "invalid_json");
        Self::new(
            "storage",
            "format",
            "Stored data or provider response has an invalid format.",
        )
    }
}
pub fn id() -> String {
    Uuid::new_v4().to_string()
}
pub fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}
pub fn valid_id(id: &str) -> Result<()> {
    Uuid::parse_str(id)
        .map(|_| ())
        .map_err(|_| AppError::input("Invalid entry identifier."))
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Transcription {
    pub provider: String,
    pub model: String,
    pub prompt: String,
    pub temperature: Option<f64>,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptionMode {
    #[default]
    Batch,
    Realtime,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RealtimeOutcome {
    pub result: Result<String>,
    /// Time from Stop until all streamed audio has a final transcript.
    pub finalize_ms: u64,
}
pub fn validate_realtime(t: &Transcription) -> Result<()> {
    if t.provider != "elevenlabs" || t.model != "scribe_v2" {
        return Err(AppError::input(
            "Real-time recording requires ElevenLabs. Imports and retries use Scribe v2.",
        ));
    }
    validate_context(t)?;
    if context_terms(t).len() > 50 {
        return Err(AppError::input(
            "Real-time transcription supports at most 50 vocabulary phrases.",
        ));
    }
    Ok(())
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Preset {
    pub id: String,
    pub revision: u64,
    pub name: String,
    pub description: String,
    pub provider: String,
    pub model: String,
    pub system: String,
    pub user: String,
    pub temperature: Option<f64>,
    #[serde(default)]
    pub reasoning: Option<String>,
}
impl Preset {
    pub fn seed() -> Self {
        Self {
            id: "00000000-0000-4000-8000-000000000001".into(),
            revision: 1,
            name: "Conservative cleanup".into(),
            description:
                "Remove transcription artifacts while preserving meaning and conversational voice."
                    .into(),
            provider: "openai".into(),
            model: "gpt-6-luna".into(),
            system: include_str!("../../prompts/conservative-cleanup.system.md").into(),
            user: include_str!("../../prompts/conservative-cleanup.user.md").into(),
            temperature: None,
            reasoning: None,
        }
    }
    pub fn validate(&self) -> Result<()> {
        valid_id(&self.id)?;
        if self.name.trim().is_empty()
            || self.name.len() > 100
            || self.description.len() > 4000
            || self.system.len() > 100_000
            || self.user.len() > 100_000
        {
            return Err(AppError::input("Preset name or prompt length is invalid."));
        }
        if self.user.matches("{{input}}").count() != 1 {
            return Err(AppError::input(
                "The user prompt must contain exactly one {{input}} placeholder.",
            ));
        }
        let m = model(&self.provider, &self.model, "transformation")?;
        let effort = self.reasoning.as_deref().unwrap_or(m.default_reasoning);
        if !m.reasoning.contains(&effort) {
            return Err(AppError::input(
                "Select a supported thinking level for this model.",
            ));
        }
        validate_temperature(
            self.temperature,
            if matches!(effort, "none" | "off") {
                m.temperature
            } else {
                None
            },
        )?;
        Ok(())
    }
    pub fn render(&self, input: &str) -> Result<String> {
        self.validate()?;
        Ok(self.user.replacen("{{input}}", input, 1))
    }
}
/// One editable transformation. Legacy preset identity/metadata stay internal for compatibility.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Transformation {
    pub revision: u64,
    pub provider: String,
    pub model: String,
    pub system: String,
    pub user: String,
    pub temperature: Option<f64>,
    #[serde(default)]
    pub reasoning: Option<String>,
}
impl From<Preset> for Transformation {
    fn from(p: Preset) -> Self {
        Self {
            revision: p.revision,
            provider: p.provider,
            model: p.model,
            system: p.system,
            user: p.user,
            temperature: p.temperature,
            reasoning: p.reasoning,
        }
    }
}
impl Transformation {
    pub fn with_metadata(self, mut legacy: Preset) -> Result<Preset> {
        legacy.provider = self.provider;
        legacy.model = self.model;
        legacy.system = self.system;
        legacy.user = self.user;
        legacy.temperature = self.temperature;
        legacy.reasoning = self.reasoning;
        legacy.validate()?;
        Ok(legacy)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Sounds {
    pub started: bool,
    pub stopped: bool,
    pub canceled: bool,
    pub transcribed: bool,
    pub transformed: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Shortcuts {
    pub toggle: String,
    pub cancel: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub transcription_mode: TranscriptionMode,
    pub revision: u64,
    pub schema: u32,
    pub microphone: Option<String>,
    pub transcription: Transcription,
    pub transform: bool,
    pub preset_id: String,
    pub compression: bool,
    pub silence_removal: bool,
    pub copy_raw: bool,
    pub copy_transformed: bool,
    pub sounds: Sounds,
    pub keep_audio: Option<u32>,
    pub theme: String,
    pub topmost: bool,
    pub autostart: bool,
    pub close_to_tray: bool,
    #[serde(default = "diagnostic_logging_default")]
    pub diagnostic_logging: bool,
    #[serde(default = "diagnostic_logging_default")]
    pub check_updates: bool,
    pub shortcuts: Shortcuts,
}
fn diagnostic_logging_default() -> bool {
    true
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            transcription_mode: TranscriptionMode::Batch,
            revision: 1,
            schema: 1,
            microphone: None,
            transcription: Transcription {
                provider: "openai".into(),
                model: "gpt-transcribe".into(),
                prompt: String::new(),
                temperature: None,
            },
            transform: false,
            preset_id: Preset::seed().id,
            compression: true,
            silence_removal: false,
            copy_raw: true,
            copy_transformed: true,
            sounds: Sounds {
                started: true,
                stopped: true,
                canceled: true,
                transcribed: true,
                transformed: true,
            },
            keep_audio: Some(100),
            theme: "system".into(),
            topmost: false,
            autostart: true,
            close_to_tray: true,
            diagnostic_logging: true,
            check_updates: true,
            shortcuts: Shortcuts {
                toggle: "Ctrl+Alt+Space".into(),
                cancel: "Ctrl+Alt+Escape".into(),
            },
        }
    }
}
impl Settings {
    pub fn validate(&self) -> Result<()> {
        if self.transcription_mode == TranscriptionMode::Realtime {
            validate_realtime(&self.transcription)?;
        }
        let m = model(
            &self.transcription.provider,
            &self.transcription.model,
            "transcription",
        )?;
        validate_temperature(self.transcription.temperature, m.temperature)?;
        if !m.prompt && !self.transcription.prompt.is_empty() {
            return Err(AppError::input(
                "This model does not support a context prompt.",
            ));
        }
        validate_context(&self.transcription)?;
        if self.transcription.prompt.len() > 4000 {
            return Err(AppError::input(
                "Context prompt is too long (maximum 4000 bytes).",
            ));
        }
        if self.keep_audio == Some(0) || self.keep_audio.is_some_and(|n| n > 100_000) {
            return Err(AppError::input(
                "Audio retention must be between 1 and 100000, or keep everything.",
            ));
        }
        if !["system", "day", "night"].contains(&self.theme.as_str()) {
            return Err(AppError::input("Invalid theme."));
        }
        if !self.shortcuts.toggle.trim().is_empty()
            && self
                .shortcuts
                .toggle
                .trim()
                .eq_ignore_ascii_case(self.shortcuts.cancel.trim())
        {
            return Err(AppError::input("Shortcuts must be distinct."));
        }
        Ok(())
    }
}
pub fn context_terms(t: &Transcription) -> Vec<&str> {
    t.prompt
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect()
}
pub fn validate_context(t: &Transcription) -> Result<()> {
    if t.provider == "elevenlabs" {
        let terms = context_terms(t);
        if terms.len() > 100
            || terms.iter().any(|s| {
                s.chars().count() >= 50
                    || s.split_whitespace().count() > 5
                    || s.contains(['<', '>', '{', '}', '[', ']', '\\'])
            })
        {
            return Err(AppError::input("Use at most 100 vocabulary phrases, fewer than 50 characters and at most five words each. Angle brackets, braces, brackets and backslashes are unsupported."));
        }
    }
    if t.provider == "deepgram" && t.prompt.len() > 500 {
        return Err(AppError::input(
            "Vocabulary hints exceed the conservative 500-byte token budget.",
        ));
    }
    Ok(())
}
fn validate_temperature(value: Option<f64>, range: Option<[f64; 2]>) -> Result<()> {
    if let Some(v) = value {
        if range.is_none_or(|r| !v.is_finite() || v < r[0] || v > r[1]) {
            return Err(AppError::input(
                "Temperature is unsupported or outside the model's range.",
            ));
        }
    }
    Ok(())
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    #[serde(default)]
    pub transcription_mode: TranscriptionMode,
    pub schema: u32,
    pub transcription: Transcription,
    pub preset: Option<Preset>,
    pub compression: bool,
    pub silence_removal: bool,
    pub copy_raw: bool,
    pub copy_transformed: bool,
}
impl Snapshot {
    pub fn capture(s: &Settings, p: Preset) -> Self {
        Self {
            transcription_mode: s.transcription_mode,
            schema: 1,
            transcription: s.transcription.clone(),
            preset: s.transform.then_some(p),
            compression: s.compression,
            silence_removal: s.silence_removal,
            copy_raw: s.copy_raw,
            copy_transformed: s.copy_transformed,
        }
    }
}
fn recording_page_size() -> u32 {
    50
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecordingQuery {
    pub search: String,
    pub sort: String,
    pub descending: bool,
    pub offset: u32,
    #[serde(default = "recording_page_size")]
    pub page_size: u32,
}
#[derive(Serialize)]
pub struct RecordingPage {
    pub entries: Vec<Entry>,
    pub total: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub created: String,
    pub source: String,
    pub title: String,
    pub subtitle: String,
    pub subtitle_manual: bool,
    pub revision: u64,
    pub audio: Option<String>,
    pub duration: f64,
    pub bytes: u64,
    pub transcript: Option<String>,
    pub transcript_revision: u64,
    pub transformed: Option<String>,
    pub transformed_revision: Option<u64>,
    #[serde(default)]
    pub transcription_ms: Option<u64>,
    #[serde(default)]
    pub transformation_ms: Option<u64>,
    pub transcript_model: Option<String>,
    pub transform_model: Option<String>,
}
impl Entry {
    pub fn new(
        id: String,
        source: &str,
        title: String,
        audio: Option<String>,
        duration: f64,
        bytes: u64,
    ) -> Self {
        Self {
            id,
            created: now(),
            source: source.into(),
            title,
            subtitle: String::new(),
            subtitle_manual: false,
            revision: 1,
            audio,
            duration,
            bytes,
            transcript: None,
            transcript_revision: 0,
            transformed: None,
            transformed_revision: None,
            transcription_ms: None,
            transformation_ms: None,
            transcript_model: None,
            transform_model: None,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Job {
    #[serde(default)]
    pub realtime: Option<RealtimeOutcome>,
    pub id: String,
    pub entry_id: String,
    pub seq: i64,
    pub kind: String,
    pub stage: String,
    pub status: String,
    pub snapshot: Snapshot,
    pub input: Option<String>,
    pub input_revision: u64,
    pub remote_id: Option<String>,
    pub error: Option<AppError>,
    pub delivery: String,
    pub eligible: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct JobView {
    pub id: String,
    pub entry_id: String,
    pub seq: i64,
    pub kind: String,
    pub stage: String,
    pub status: String,
    pub remote_id: Option<String>,
    pub error: Option<AppError>,
    pub delivery: String,
    pub eligible: bool,
}
impl From<Job> for JobView {
    fn from(j: Job) -> Self {
        Self {
            id: j.id,
            entry_id: j.entry_id,
            seq: j.seq,
            kind: j.kind,
            stage: j.stage,
            status: j.status,
            remote_id: j.remote_id,
            error: j.error,
            delivery: j.delivery,
            eligible: j.eligible,
        }
    }
}
impl Job {
    pub fn can_poll_transcription(&self) -> bool {
        self.stage == "transcribing"
            && self.snapshot.transcription.provider == "assemblyai"
            && self.remote_id.as_ref().is_some_and(|id| !id.is_empty())
    }
    pub fn requires_explicit_retry(&self) -> bool {
        self.stage == "transforming"
            || (self.stage == "transcribing"
                && !self.can_poll_transcription()
                && !(self.snapshot.transcription_mode == TranscriptionMode::Realtime
                    && self.realtime.as_ref().is_some_and(|r| r.result.is_ok())))
    }
    pub fn new(entry_id: String, kind: &str, mut snapshot: Snapshot, eligible: bool) -> Self {
        if kind != "initial" {
            snapshot.transcription_mode = TranscriptionMode::Batch;
        }
        Self {
            realtime: None,
            id: id(),
            entry_id,
            seq: 0,
            kind: kind.into(),
            stage: "queued".into(),
            status: "queued".into(),
            snapshot,
            input: None,
            input_revision: 0,
            remote_id: None,
            error: None,
            delivery: "pending".into(),
            eligible,
        }
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct Model {
    pub provider: &'static str,
    pub id: &'static str,
    pub stage: &'static str,
    pub prompt: bool,
    pub context_field: Option<&'static str>,
    pub temperature: Option<[f64; 2]>,
    pub max_bytes: u64,
    pub context: u32,
    pub output: u32,
    pub label: &'static str,
    pub description: &'static str,
    pub reasoning: Vec<&'static str>,
    pub default_reasoning: &'static str,
}
pub fn catalog() -> Vec<Model> {
    vec![
        Model {
            provider: "openai",
            id: "gpt-transcribe",
            stage: "transcription",
            prompt: true,
            context_field: Some("prompt"),
            temperature: None,
            max_bytes: 25_000_000,
            context: 0,
            output: 0,
            label: "",
            description: "",
            reasoning: vec![],
            default_reasoning: "none",
        },
        Model {
            provider: "openai",
            id: "whisper-1",
            stage: "transcription",
            prompt: true,
            context_field: Some("prompt"),
            temperature: Some([0., 1.]),
            max_bytes: 25_000_000,
            context: 0,
            output: 0,
            label: "",
            description: "",
            reasoning: vec![],
            default_reasoning: "none",
        },
        Model {
            provider: "assemblyai",
            id: "universal-3-5-pro",
            stage: "transcription",
            prompt: true,
            context_field: Some("prompt"),
            temperature: None,
            max_bytes: 2_200_000_000,
            context: 0,
            output: 0,
            label: "",
            description: "",
            reasoning: vec![],
            default_reasoning: "none",
        },
        Model {
            provider: "elevenlabs",
            id: "scribe_v2",
            stage: "transcription",
            prompt: true,
            context_field: Some("keyterms"),
            temperature: Some([0., 2.]),
            max_bytes: 5_000_000_000,
            context: 0,
            output: 0,
            label: "",
            description: "",
            reasoning: vec![],
            default_reasoning: "none",
        },
        Model {
            provider: "deepgram",
            id: "nova-3",
            stage: "transcription",
            prompt: true,
            context_field: Some("keyterm"),
            temperature: None,
            max_bytes: 2_000_000_000,
            context: 0,
            output: 0,
            label: "",
            description: "",
            reasoning: vec![],
            default_reasoning: "none",
        },
        text_model(
            "openai",
            "gpt-6-luna",
            "GPT-6 Luna · fast / recommended",
            "Efficient everyday transcript cleanup.",
            &["none", "low", "medium", "high", "xhigh", "max"],
            "none",
        ),
        text_model(
            "openai",
            "gpt-5-nano",
            "GPT-5 nano · legacy / economical",
            "Small legacy option for straightforward cleanup; compare accuracy on your speech.",
            &["minimal", "low", "medium", "high"],
            "minimal",
        ),
        text_model(
            "openai",
            "gpt-5-mini",
            "GPT-5 mini · legacy / balanced",
            "A legacy choice for well-defined editing tasks.",
            &["minimal", "low", "medium", "high"],
            "minimal",
        ),
        text_model(
            "openai",
            "gpt-5.4-mini",
            "GPT-5.4 mini · fast / capable",
            "A compact model with optional thinking.",
            &["none", "low", "medium", "high", "xhigh"],
            "none",
        ),
        text_model(
            "openai",
            "gpt-6.1-sol",
            "GPT-6.1 Sol · more capable",
            "For more demanding edits; reasoning increases time and cost.",
            &["low", "medium", "high", "xhigh", "max"],
            "low",
        ),
        text_model(
            "openai",
            "gpt-6-astra",
            "GPT-6 Astra · highest capability",
            "For difficult, quality-first work; start with low thinking effort.",
            &["low", "medium", "high", "xhigh", "max"],
            "low",
        ),
        text_model(
            "anthropic",
            "claude-haiku-5-5",
            "Claude Haiku 5.5 · fastest Claude",
            "A compact Claude option for routine cleanup.",
            &["off", "low", "medium", "high"],
            "off",
        ),
        text_model(
            "anthropic",
            "claude-sonnet-5-5",
            "Claude Sonnet 5.5 · balanced",
            "Balances response speed and intelligence.",
            &["off", "low", "medium", "high"],
            "off",
        ),
        text_model(
            "anthropic",
            "claude-opus-5-5",
            "Claude Opus 5.5 · more capable",
            "A more capable Claude option; thinking is always enabled.",
            &["low", "medium", "high"],
            "low",
        ),
    ]
}
fn text_model(
    provider: &'static str,
    id: &'static str,
    label: &'static str,
    description: &'static str,
    reasoning: &[&'static str],
    default_reasoning: &'static str,
) -> Model {
    Model {
        provider,
        id,
        label,
        description,
        stage: "transformation",
        prompt: true,
        context_field: None,
        temperature: if id == "gpt-6-luna" {
            Some([0., 2.])
        } else {
            None
        },
        max_bytes: 0,
        context: if provider == "anthropic" {
            1_000_000
        } else if id.starts_with("gpt-5") {
            400_000
        } else {
            1_050_000
        },
        output: 128_000,
        reasoning: reasoning.to_vec(),
        default_reasoning,
    }
}
pub fn model(provider: &str, id: &str, stage: &str) -> Result<Model> {
    catalog()
        .into_iter()
        .find(|m| m.provider == provider && m.id == id && m.stage == stage)
        .ok_or_else(|| AppError::input("Select a supported provider and model."))
}
pub fn auto_copy(j: &Job) -> bool {
    j.eligible
        && j.kind == "initial"
        && if j.snapshot.preset.is_some() {
            j.snapshot.copy_transformed
        } else {
            j.snapshot.copy_raw
        }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shortcuts_allow_removal_and_ignore_legacy_stop() {
        let mut json = serde_json::to_value(Settings::default()).unwrap();
        json["shortcuts"]["stop"] = serde_json::json!("Ctrl+Alt+Enter");
        let mut settings: Settings = serde_json::from_value(json).unwrap();
        assert!(serde_json::to_value(&settings).unwrap()["shortcuts"]
            .get("stop")
            .is_none());
        settings.shortcuts.toggle.clear();
        settings.shortcuts.cancel.clear();
        assert!(settings.validate().is_ok());
        settings.shortcuts.toggle = "Ctrl+Alt+Space".into();
        assert!(settings.validate().is_ok());
        settings.shortcuts.cancel = " ctrl+alt+space ".into();
        assert!(settings.validate().is_err());
    }

    #[test]
    fn template_is_literal() {
        let p = Preset::seed();
        assert_eq!(
            p.render("{{input}} <script>").unwrap(),
            "<transcript>\n{{input}} <script>\n</transcript>\n"
        );
        let mut bad = p;
        bad.user = "{{input}}{{input}}".into();
        assert!(bad.validate().is_err());
    }
    #[test]
    fn copy_matrix() {
        for transform in [false, true] {
            for raw in [false, true] {
                for cleaned in [false, true] {
                    let s = Settings {
                        transform,
                        copy_raw: raw,
                        copy_transformed: cleaned,
                        ..Settings::default()
                    };
                    let snap = Snapshot::capture(&s, Preset::seed());
                    let mut j = Job::new(id(), "initial", snap, true);
                    assert_eq!(auto_copy(&j), if transform { cleaned } else { raw });
                    for kind in ["import", "transcribe", "transform"] {
                        j.kind = kind.into();
                        assert!(!auto_copy(&j));
                    }
                    j.kind = "initial".into();
                    j.eligible = false;
                    assert!(!auto_copy(&j));
                }
            }
        }
    }
}
