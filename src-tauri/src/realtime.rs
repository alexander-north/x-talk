//! Native streaming, isolated from the real-time microphone callback and disk writer.
use crate::{credentials::CredentialStore, domain::*};
use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD, Engine};
use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};

pub const MODEL: &str = "scribe_v2_realtime";
const ENDPOINT: &str = "wss://api.elevenlabs.io/v1/speech-to-text/realtime";
const FINAL_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_TEXT: usize = 1_000_000;
pub type Preview = Arc<dyn Fn(LivePreview) + Send + Sync>;

#[derive(Clone, Default, Serialize)]
pub struct LivePreview {
    pub text: String,
    pub status: String,
    pub error: Option<AppError>,
}
pub fn failure(category: &str, message: &str) -> AppError {
    AppError::new("transcribing", category, message)
}

#[async_trait]
pub trait StreamingProvider: Send + Sync {
    async fn stream(
        &self,
        rate: u32,
        config: Transcription,
        audio: mpsc::Receiver<Vec<i16>>,
        preview: Preview,
    ) -> Result<String>;
}
pub struct ElevenLabs {
    pub keys: Arc<dyn CredentialStore>,
}
#[async_trait]
impl StreamingProvider for ElevenLabs {
    async fn stream(
        &self,
        rate: u32,
        config: Transcription,
        audio: mpsc::Receiver<Vec<i16>>,
        preview: Preview,
    ) -> Result<String> {
        validate_realtime(&config)?;
        let key = self.keys.get("elevenlabs")?;
        connect(ENDPOINT, &key, rate, &config, audio, preview).await
    }
}

/// The sender never waits for network I/O. Overflow fails streaming rather than dropping words.
pub struct AudioFeed {
    tx: mpsc::Sender<Vec<i16>>,
    cancel: watch::Sender<bool>,
    fault: Arc<Mutex<Option<AppError>>>,
    channels: usize,
    sum: f32,
    channel: usize,
    chunk: Vec<i16>,
    chunk_size: usize,
}
impl AudioFeed {
    pub fn push(&mut self, sample: f32) {
        self.sum += if sample.is_finite() { sample } else { 0.0 };
        self.channel += 1;
        if self.channel == self.channels {
            self.chunk.push(
                ((self.sum / self.channels as f32).clamp(-1.0, 1.0) * i16::MAX as f32).round()
                    as i16,
            );
            self.sum = 0.0;
            self.channel = 0;
            if self.chunk.len() >= self.chunk_size {
                self.flush();
            }
        }
    }
    pub fn flush(&mut self) {
        if self.chunk.is_empty() {
            return;
        }
        let chunk = std::mem::replace(&mut self.chunk, Vec::with_capacity(self.chunk_size));
        if let Err(mpsc::error::TrySendError::Full(_)) = self.tx.try_send(chunk) {
            if let Ok(mut fault) = self.fault.lock() {
                *fault = Some(failure("stream_backpressure", "Real-time transcription could not keep up. Original audio is retained; retry transcription from Recordings."));
            }
            let _ = self.cancel.send(true);
        }
    }
}
pub struct LiveSession {
    cancel: watch::Sender<bool>,
    result: std::sync::mpsc::Receiver<Result<String>>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl LiveSession {
    pub fn start(
        provider: Arc<dyn StreamingProvider>,
        rate: u32,
        channels: u16,
        config: Transcription,
        preview: Preview,
    ) -> Result<(Self, AudioFeed)> {
        if !supported_rate(rate) || !matches!(channels, 1 | 2) {
            return Err(AppError::input(
                "Microphone format is unsupported for real-time transcription.",
            ));
        }
        let (tx, audio) = mpsc::channel(100); // ten seconds of 100 ms audio
        let (cancel, mut canceled) = watch::channel(false);
        let (result_tx, result) = std::sync::mpsc::sync_channel(1);
        let fault = Arc::new(Mutex::new(None));
        let worker_fault = fault.clone();
        let thread = std::thread::Builder::new().name("realtime-transcription".into()).spawn(move || {
            let result = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                Ok(runtime) => runtime.block_on(async {
                    tokio::select! {
                        biased;
                        _ = canceled.changed() => Err(failure("canceled", "Real-time transcription stopped.")),
                        result = provider.stream(rate, config, audio, preview.clone()) => result,
                    }
                }),
                Err(_) => Err(failure("stream_start", "Could not start real-time transcription.")),
            };
            let result = worker_fault.lock().ok().and_then(|e| e.clone()).map_or(result, Err);
            if let Err(e) = &result {
                preview(LivePreview { status: "failed".into(), error: Some(e.clone()), ..Default::default() });
            }
            let _ = result_tx.send(result);
        })?;
        Ok((
            Self {
                cancel: cancel.clone(),
                result,
                thread: Some(thread),
            },
            AudioFeed {
                tx,
                cancel,
                fault,
                channels: channels as usize,
                sum: 0.0,
                channel: 0,
                chunk: Vec::with_capacity(rate as usize / 10),
                chunk_size: rate as usize / 10,
            },
        ))
    }
    pub fn finish(mut self, stopped: Instant) -> RealtimeOutcome {
        let result = self.result.recv_timeout(FINAL_TIMEOUT + Duration::from_secs(5)).unwrap_or_else(|_| Err(failure("timeout", "Real-time finalization timed out. Original audio is retained; retry from Recordings.")));
        let _ = self.cancel.send(true);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        RealtimeOutcome {
            result,
            finalize_ms: stopped.elapsed().as_millis() as u64,
        }
    }
}
impl Drop for LiveSession {
    fn drop(&mut self) {
        let _ = self.cancel.send(true);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
pub fn supported_rate(rate: u32) -> bool {
    matches!(rate, 8000 | 16000 | 22050 | 24000 | 44100 | 48000)
}

fn transport(_: tokio_tungstenite::tungstenite::Error) -> AppError {
    failure("transport", "Real-time connection failed. Check the connection and API key, then retry from Recordings. Original audio is retained.")
}
fn frame(samples: &[i16], rate: u32, commit: bool) -> Message {
    let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    Message::Text(json!({"message_type":"input_audio_chunk", "audio_base_64":STANDARD.encode(bytes), "sample_rate":rate,"commit":commit}).to_string().into())
}

#[derive(Default)]
struct Transcript {
    committed: String,
    partial: String,
}
impl Transcript {
    fn update(&mut self, value: &Value, pending: bool) -> Result<bool> {
        let kind = value["message_type"].as_str().unwrap_or("");
        match kind {
            "partial_transcript" | "committed_transcript" => {
                let text = value["text"].as_str().ok_or_else(|| failure("malformed", "Real-time transcript was malformed."))?;
                if self.committed.len() + text.len() > MAX_TEXT { return Err(failure("size", "Real-time transcript exceeded the supported text limit.")); }
                if kind == "partial_transcript" { self.partial = text.into(); return Ok(false); }
                if !pending { return Err(failure("stream_protocol", "An unexpected transcript boundary was received. Retry from the saved recording.")); }
                if !text.trim().is_empty() {
                    if !self.committed.is_empty() { self.committed.push(' '); }
                    self.committed.push_str(text.trim());
                }
                self.partial.clear();
                Ok(true)
            }
            "session_started" | "warning" => Ok(false),
            // Never echo provider errors, which can contain credentials or transcript text.
            _ => Err(failure("stream_provider", "Real-time transcription was interrupted by the provider. Check your key, quota and connection; retry from the saved recording.")),
        }
    }
    fn preview(&self) -> LivePreview {
        LivePreview {
            text: [self.committed.as_str(), self.partial.as_str()]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" "),
            status: "streaming".into(),
            error: None,
        }
    }
}

async fn connect(
    endpoint: &str,
    key: &str,
    rate: u32,
    config: &Transcription,
    mut audio: mpsc::Receiver<Vec<i16>>,
    preview: Preview,
) -> Result<String> {
    let mut url = reqwest::Url::parse(endpoint)
        .map_err(|_| AppError::input("Invalid real-time endpoint."))?;
    {
        let mut query = url.query_pairs_mut();
        query
            .append_pair("model_id", MODEL)
            .append_pair("audio_format", &format!("pcm_{rate}"))
            .append_pair("language_code", "en")
            .append_pair("commit_strategy", "manual");
        for term in context_terms(config) {
            query.append_pair("keyterms", term);
        }
    }
    let mut request = url.as_str().into_client_request().map_err(transport)?;
    request.headers_mut().insert(
        "xi-api-key",
        key.parse()
            .map_err(|_| AppError::input("Invalid ElevenLabs API key."))?,
    );
    let (mut socket, _) = tokio::time::timeout(
        Duration::from_secs(10),
        tokio_tungstenite::connect_async(request),
    )
    .await
    .map_err(|_| failure("timeout", "Real-time connection timed out."))?
    .map_err(transport)?;
    let mut transcript = Transcript::default();
    let mut started = false;
    let mut pending = false;
    let mut ending = false;
    let mut segment = 0usize;
    let mut silence = 0usize;
    let mut deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(deadline), if !started || pending => {
                return Err(failure("timeout", "The provider did not confirm the final transcript. Original audio is retained; retry from Recordings."));
            }
            message = socket.next() => {
                let message = message.ok_or_else(|| failure("transport", "Real-time connection closed before finalization."))?.map_err(transport)?;
                match message {
                    Message::Text(text) => {
                        let value: Value = serde_json::from_str(&text).map_err(|_| failure("malformed", "Invalid real-time provider response."))?;
                        if value["message_type"] == "session_started" { started = true; }
                        let committed = transcript.update(&value, pending)?;
                        preview(transcript.preview());
                        if committed {
                            pending = false; segment = 0; silence = 0;
                            if ending { break; }
                        }
                    }
                    Message::Ping(data) => {
                        tokio::time::timeout(Duration::from_secs(5), socket.send(Message::Pong(data))).await.map_err(|_| failure("timeout", "Real-time connection stalled."))?.map_err(transport)?;
                    }
                    Message::Close(_) => return Err(failure("transport", "Real-time connection closed before finalization.")),
                    Message::Pong(_) => {},
                    _ => return Err(failure("malformed", "Unexpected real-time provider response.")),
                }
            }
            chunk = audio.recv(), if started && !pending && !ending => {
                let (samples, commit) = if let Some(samples) = chunk {
                    segment += samples.len();
                    for sample in &samples { if sample.unsigned_abs() < 100 { silence += 1; } else { silence = 0; } }
                    // Keep segments below the provider's automatic 36s boundary. Prefer pauses.
                    let commit = (segment >= rate as usize * 20 && silence >= rate as usize / 2) || segment >= rate as usize * 30;
                    (samples, commit)
                } else {
                    ending = true;
                    if segment == 0 { break; }
                    // The service starts processing at 2s. Pad very short final segments only.
                    (vec![0; (rate as usize * 2).saturating_sub(segment)], true)
                };
                tokio::time::timeout(Duration::from_secs(5), socket.send(frame(&samples, rate, commit))).await
                    .map_err(|_| failure("timeout", "Real-time audio upload stalled."))?.map_err(transport)?;
                if commit { pending = true; deadline = tokio::time::Instant::now() + FINAL_TIMEOUT; }
            }
        }
    }
    // Do not let the closing handshake delay already-confirmed text.
    if transcript.committed.trim().is_empty() {
        return Err(failure(
            "no_speech",
            "No speech detected. Original audio is retained.",
        ));
    }
    Ok(transcript.committed)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> Transcription {
        Transcription {
            provider: "elevenlabs".into(),
            model: "scribe_v2".into(),
            prompt: "x-talk\nElevenLabs".into(),
            temperature: None,
        }
    }
    #[test]
    fn partials_replace_and_commits_preserve_repeated_words() {
        let mut text = Transcript::default();
        for word in ["hel", "hello"] {
            text.update(
                &json!({"message_type":"partial_transcript","text":word}),
                false,
            )
            .unwrap();
        }
        assert_eq!(text.preview().text, "hello");
        for _ in 0..2 {
            text.update(
                &json!({"message_type":"committed_transcript","text":"hello"}),
                true,
            )
            .unwrap();
        }
        assert_eq!(text.preview().text, "hello hello");
        assert!(text
            .update(
                &json!({"message_type":"committed_transcript","text":"lost"}),
                false
            )
            .is_err());
        let error = text
            .update(
                &json!({"message_type":"auth_error","error":"secret credential"}),
                false,
            )
            .unwrap_err();
        assert!(!error.message.contains("secret"));
        assert!(text
            .update(
                &json!({"message_type":"partial_transcript","text":42}),
                false
            )
            .is_err());
    }
    #[tokio::test]
    async fn websocket_streams_pcm_and_waits_for_each_final_commit() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("ws://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_hdr_async(
                socket,
                |request: &tokio_tungstenite::tungstenite::handshake::server::Request, response| {
                    assert_eq!(request.headers()["xi-api-key"], "test-only");
                    let uri = request.uri().to_string();
                    assert!(uri.contains("model_id=scribe_v2_realtime"));
                    assert!(uri.contains("keyterms=x-talk"));
                    assert!(uri.contains("commit_strategy=manual"));
                    Ok(response)
                },
            )
            .await
            .unwrap();
            socket
                .send(Message::Text(
                    json!({"message_type":"session_started"}).to_string().into(),
                ))
                .await
                .unwrap();
            let mut commits = 0;
            let mut sample_count = 0;
            loop {
                let Some(Ok(Message::Text(text))) = socket.next().await else {
                    break;
                };
                let v: Value = serde_json::from_str(&text).unwrap();
                assert_eq!(v["sample_rate"], 16000);
                let bytes = STANDARD
                    .decode(v["audio_base_64"].as_str().unwrap())
                    .unwrap();
                sample_count += bytes.len() / 2;
                if v["commit"] == true {
                    commits += 1;
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    socket.send(Message::Text(json!({"message_type":"committed_transcript","text":if commits==1 {"First sentence."} else {"Last words."}}).to_string().into())).await.unwrap();
                    if commits == 2 {
                        break;
                    }
                } else {
                    socket
                        .send(Message::Text(
                            json!({"message_type":"partial_transcript","text":"draft"})
                                .to_string()
                                .into(),
                        ))
                        .await
                        .unwrap();
                }
            }
            (commits, sample_count)
        });
        let (tx, rx) = mpsc::channel(100);
        let producer = tokio::spawn(async move {
            // Force a 30s continuous-speech boundary, then a 1s final segment padded to 2s.
            for _ in 0..310 {
                tx.send(vec![1000; 1600]).await.unwrap();
            }
        });
        let previews = Arc::new(Mutex::new(Vec::new()));
        let saved = previews.clone();
        let result = connect(
            &endpoint,
            "test-only",
            16000,
            &config(),
            rx,
            Arc::new(move |p| saved.lock().unwrap().push(p.text)),
        )
        .await
        .unwrap();
        producer.await.unwrap();
        let (commits, samples) = server.await.unwrap();
        assert_eq!(commits, 2);
        assert_eq!(samples, 32 * 16000);
        assert_eq!(result, "First sentence. Last words.");
        assert!(previews.lock().unwrap().iter().any(|p| p == "draft"));
    }
    #[tokio::test]
    async fn premature_close_never_promotes_partial_text() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("ws://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(socket).await.unwrap();
            for value in [
                json!({"message_type":"session_started"}),
                json!({"message_type":"partial_transcript","text":"Not final"}),
            ] {
                socket
                    .send(Message::Text(value.to_string().into()))
                    .await
                    .unwrap();
            }
            socket.close(None).await.unwrap();
        });
        let (_tx, rx) = mpsc::channel(2);
        let error = connect(&endpoint, "test", 16000, &config(), rx, Arc::new(|_| {}))
            .await
            .unwrap_err();
        assert_eq!(error.category, "transport");
        server.await.unwrap();
    }
    struct Inspect;
    #[async_trait]
    impl StreamingProvider for Inspect {
        async fn stream(
            &self,
            _: u32,
            _: Transcription,
            mut audio: mpsc::Receiver<Vec<i16>>,
            _: Preview,
        ) -> Result<String> {
            let mut samples = Vec::new();
            while let Some(chunk) = audio.recv().await {
                samples.extend(chunk);
            }
            assert_eq!(samples, vec![0, i16::MAX, -i16::MAX]);
            Ok("Confirmed.".into())
        }
    }
    #[test]
    fn stereo_downmix_final_flush_and_normal_stop() {
        let (session, mut feed) =
            LiveSession::start(Arc::new(Inspect), 16000, 2, config(), Arc::new(|_| {})).unwrap();
        for s in [1.0, -1.0, 1.0, 1.0, -1.0, -1.0] {
            feed.push(s);
        }
        feed.flush();
        drop(feed);
        assert_eq!(session.finish(Instant::now()).result.unwrap(), "Confirmed.");
    }
    struct Stalled;
    #[async_trait]
    impl StreamingProvider for Stalled {
        async fn stream(
            &self,
            _: u32,
            _: Transcription,
            _: mpsc::Receiver<Vec<i16>>,
            _: Preview,
        ) -> Result<String> {
            std::future::pending().await
        }
    }
    #[test]
    fn overflow_fails_without_blocking_and_cancel_joins_worker() {
        let (session, mut feed) =
            LiveSession::start(Arc::new(Stalled), 8000, 1, config(), Arc::new(|_| {})).unwrap();
        for _ in 0..101 * 800 {
            feed.push(0.2);
        }
        drop(feed);
        assert_eq!(
            session.finish(Instant::now()).result.unwrap_err().category,
            "stream_backpressure"
        );
        let (session, _feed) =
            LiveSession::start(Arc::new(Stalled), 8000, 1, config(), Arc::new(|_| {})).unwrap();
        let start = Instant::now();
        drop(session);
        assert!(start.elapsed() < Duration::from_secs(2));
    }
    #[test]
    fn legacy_settings_and_mode_specific_validation() {
        let mut value = serde_json::to_value(Settings::default()).unwrap();
        value.as_object_mut().unwrap().remove("transcription_mode");
        let mut settings: Settings = serde_json::from_value(value).unwrap();
        assert_eq!(settings.transcription_mode, TranscriptionMode::Batch);
        settings.transcription_mode = TranscriptionMode::Realtime;
        assert!(settings.validate().is_err());
        settings.transcription = config();
        assert!(settings.validate().is_ok());
        settings.transcription.prompt = vec!["term"; 51].join("\n");
        assert!(settings.validate().is_err());
        settings.transcription_mode = TranscriptionMode::Batch;
        assert!(settings.validate().is_ok());
    }
}
