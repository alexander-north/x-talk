//! Explicitly enabled, isolated debug acceptance backend. Never compiled into release builds.
use crate::{
    credentials::CredentialStore,
    domain::*,
    media::{AudioInfo, MediaProcessor, PreparedAudio},
    providers::{TranscriptionProvider, TransformationProvider},
};
use async_trait::async_trait;
use std::path::Path;
pub fn enabled() -> bool {
    std::env::var("XTALK_TEST_PROVIDERS").as_deref() == Ok("1")
        && std::env::var_os("XTALK_TEST_DATA_DIR").is_some()
}
pub struct TestProviders;
#[async_trait]
impl crate::realtime::StreamingProvider for TestProviders {
    async fn stream(
        &self,
        _: u32,
        _: Transcription,
        mut audio: tokio::sync::mpsc::Receiver<Vec<i16>>,
        preview: crate::realtime::Preview,
    ) -> Result<String> {
        let mut heard = false;
        while audio.recv().await.is_some() {
            if !heard {
                heard = true;
                preview(crate::realtime::LivePreview {
                    text: "Acceptance live preview.".into(),
                    status: "streaming".into(),
                    error: None,
                });
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        Ok("Acceptance final real-time transcript.".into())
    }
}
#[async_trait]
impl MediaProcessor for TestProviders {
    async fn probe(&self, path: &Path) -> Result<AudioInfo> {
        let wav = hound::WavReader::open(path).map_err(|_| AppError::input("Invalid test WAV."))?;
        let duration = wav.duration() as f64 / wav.spec().sample_rate as f64;
        if duration <= 0. {
            return Err(AppError::input("Test recording has no samples."));
        }
        Ok(AudioInfo {
            duration,
            bytes: path.metadata()?.len(),
        })
    }
    async fn prepare(&self, path: &Path, _: &Path, _: &Snapshot) -> Result<PreparedAudio> {
        // Real device and writer, with deterministic processing even on a silent microphone.
        let info = self.probe(path).await?;
        Ok(PreparedAudio {
            path: path.into(),
            bytes: info.bytes,
            mime: "audio/wav",
        })
    }
}
#[async_trait]
impl TranscriptionProvider for TestProviders {
    async fn transcribe(
        &self,
        _: &PreparedAudio,
        _: &Job,
        _: &(dyn Fn(String) -> Result<()> + Send + Sync),
    ) -> Result<String> {
        tokio::time::sleep(std::time::Duration::from_millis(700)).await;
        Ok("Acceptance raw transcript.".into())
    }
}
#[async_trait]
impl TransformationProvider for TestProviders {
    async fn transform(&self, _: &str, _: &Preset) -> Result<String> {
        // Leave an observable transcription-only stage for native card-arrival regression tests.
        tokio::time::sleep(std::time::Duration::from_millis(2400)).await;
        Ok("Acceptance transformed output.".into())
    }
}
fn disabled_keys() -> &'static std::sync::Mutex<std::collections::HashSet<String>> {
    static KEYS: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> =
        std::sync::OnceLock::new();
    KEYS.get_or_init(Default::default)
}
impl CredentialStore for TestProviders {
    fn get(&self, provider: &str) -> Result<String> {
        if disabled_keys().lock().unwrap().contains(provider) {
            return Err(AppError::input("No test key configured."));
        }
        Ok("acceptance-placeholder".into())
    }
    fn set(&self, provider: &str, _: &str) -> Result<()> {
        disabled_keys().lock().unwrap().remove(provider);
        Ok(())
    }
    fn delete(&self, provider: &str) -> Result<()> {
        disabled_keys().lock().unwrap().insert(provider.into());
        Ok(())
    }
}
