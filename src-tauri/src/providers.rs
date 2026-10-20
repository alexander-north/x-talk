use crate::{credentials::CredentialStore, domain::*, media::PreparedAudio};
use async_trait::async_trait;
use reqwest::{
    multipart::{Form, Part},
    Body, Client, RequestBuilder,
};
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};
#[async_trait]
pub trait TranscriptionProvider: Send + Sync {
    async fn transcribe(
        &self,
        audio: &PreparedAudio,
        j: &Job,
        remote: &(dyn Fn(String) -> Result<()> + Send + Sync),
    ) -> Result<String>;
}
#[async_trait]
pub trait TransformationProvider: Send + Sync {
    async fn transform(&self, input: &str, preset: &Preset) -> Result<String>;
}
pub struct Providers {
    pub(crate) client: Client,
    pub(crate) keys: Arc<dyn CredentialStore>,
}
impl Providers {
    pub fn new(keys: Arc<dyn CredentialStore>) -> Result<Self> {
        Ok(Self {
            client: Client::builder()
                .connect_timeout(Duration::from_secs(15))
                .read_timeout(Duration::from_secs(300))
                .timeout(Duration::from_secs(900))
                .redirect(reqwest::redirect::Policy::none())
                .user_agent(format!("x-talk/{}", env!("CARGO_PKG_VERSION")))
                .build()
                .map_err(|_| {
                    AppError::new("network", "client", "Could not initialize HTTPS client.")
                })?,
            keys,
        })
    }
    pub(crate) async fn json(&self, req: RequestBuilder, stage: &str) -> Result<Value> {
        self.json_limit(req, stage, 8 * 1024 * 1024).await
    }
    pub(crate) async fn json_limit(
        &self,
        req: RequestBuilder,
        stage: &str,
        limit: usize,
    ) -> Result<Value> {
        tracing::info!(stage, "provider_request_started");
        let mut response=req.send().await.map_err(|e| {
tracing::warn!(stage, timeout = e.is_timeout(), "provider_transport_failed");
AppError::new(stage,if e.is_timeout(){
"timeout"}
else{
"transport"}
,"Provider request did not complete. Its billing outcome may be unknown; review history before retrying.")})?;
        let status = response.status();
        let request_id = response
            .headers()
            .get("x-request-id")
            .or_else(|| response.headers().get("request-id"))
            .and_then(|v| v.to_str().ok())
            .filter(|s| {
                s.len() < 150
                    && s.chars()
                        .all(|c| c.is_ascii_alphanumeric() || "-_".contains(c))
            })
            .map(str::to_owned);
        tracing::info!(
            stage,
            status = status.as_u16(),
            request_id,
            "provider_response"
        );
        if !status.is_success() {
            let(category,message)=match status.as_u16(){
401|403=>("authentication","Provider rejected the API key or account access. Replace the key or check model access."),413=>("size","Provider rejected upload size. Enable compression and retry."),429=>("rate_limit","Provider rate limit reached. Wait before retrying."),400|404|422=>("unsupported_request","Provider rejected the model or parameters. Verify model access and update x-talk if necessary."),500..=599=>("provider_unavailable","Provider is temporarily unavailable. Retry later; billing outcome may be unknown."),_=>("provider_error","Provider request failed. Review settings before retrying.")}
;
            return Err(AppError::new(stage, category, message));
        }
        let mut data = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| {
            AppError::new(
                stage,
                "transport",
                "Provider response was interrupted. The billing outcome may be unknown.",
            )
        })? {
            if data.len() + chunk.len() > limit {
                return Err(AppError::new(
                    stage,
                    "response_size",
                    "Provider response exceeded the supported size.",
                ));
            }
            data.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&data)
            .map_err(|_| AppError::new(stage, "malformed", "Provider returned malformed JSON."))
    }
    async fn body(audio: &PreparedAudio) -> Result<Body> {
        let f = tokio::fs::File::open(&audio.path).await?;
        Ok(Body::wrap_stream(tokio_util::io::ReaderStream::new(f)))
    }
    async fn part(audio: &PreparedAudio) -> Result<Part> {
        Part::stream_with_length(Self::body(audio).await?, audio.bytes)
            .file_name(if audio.mime == "audio/wav" {
                "audio.wav"
            } else {
                "audio.m4a"
            })
            .mime_str(audio.mime)
            .map_err(|_| AppError::input("Invalid audio content type."))
    }
}
#[async_trait]
impl TranscriptionProvider for Providers {
    async fn transcribe(
        &self,
        audio: &PreparedAudio,
        j: &Job,
        remote: &(dyn Fn(String) -> Result<()> + Send + Sync),
    ) -> Result<String> {
        let t = &j.snapshot.transcription;
        let m = model(&t.provider, &t.model, "transcription")?;
        if audio.bytes > m.max_bytes {
            return Err(AppError::new("transcribing","size","Upload exceeds the provider's limit. Enable compression and retry; the original is retained."));
        }
        validate_context(t)?;
        let key = self.keys.get(&t.provider)?;
        let value = match t.provider.as_str() {
            "openai" => {
                let mut form = Form::new()
                    .text("model", t.model.clone())
                    .text("response_format", "json")
                    .text(
                        if t.model == "gpt-transcribe" {
                            "languages[]"
                        } else {
                            "language"
                        },
                        "en",
                    )
                    .part("file", Self::part(audio).await?);
                if !t.prompt.is_empty() {
                    form = form.text("prompt", t.prompt.clone());
                }
                if let Some(v) = t.temperature {
                    form = form.text("temperature", v.to_string());
                }
                self.json(
                    self.client
                        .post("https://api.openai.com/v1/audio/transcriptions")
                        .bearer_auth(key)
                        .multipart(form),
                    "transcribing",
                )
                .await?
            }
            "elevenlabs" => {
                let mut form = Form::new()
                    .text("model_id", t.model.clone())
                    .text("language_code", "eng")
                    .text("diarize", "false")
                    .text("tag_audio_events", "false")
                    .part("file", Self::part(audio).await?);
                for term in context_terms(t) {
                    form = form.text("keyterms", term.to_owned());
                }
                if let Some(v) = t.temperature {
                    form = form.text("temperature", v.to_string());
                }
                self.json(
                    self.client
                        .post("https://api.elevenlabs.io/v1/speech-to-text")
                        .header("xi-api-key", key)
                        .multipart(form),
                    "transcribing",
                )
                .await?
            }
            "deepgram" => {
                let mut req = self
                    .client
                    .post("https://api.deepgram.com/v1/listen")
                    .query(&[
                        ("model", t.model.as_str()),
                        ("language", "en"),
                        ("punctuate", "true"),
                        ("diarize", "false"),
                        ("multichannel", "false"),
                    ])
                    .header("Authorization", format!("Token {key}"))
                    .header("Content-Type", audio.mime)
                    .header("Content-Length", audio.bytes)
                    .body(Self::body(audio).await?);
                for term in context_terms(t) {
                    req = req.query(&[("keyterm", term)]);
                }
                self.json(req, "transcribing").await?
            }
            "assemblyai" => {
                let remote_id = if let Some(id) = &j.remote_id {
                    id.clone()
                } else {
                    let upload = self
                        .json(
                            self.client
                                .post("https://api.assemblyai.com/v2/upload")
                                .header("authorization", &key)
                                .header("Content-Type", "application/octet-stream")
                                .header("Content-Length", audio.bytes)
                                .body(Self::body(audio).await?),
                            "transcribing",
                        )
                        .await?;
                    let url = upload
                        .get("upload_url")
                        .and_then(Value::as_str)
                        .filter(|s| {
                            s.starts_with("https://cdn.assemblyai.com/")
                                || s.starts_with("https://storage.assemblyai.com/")
                        })
                        .ok_or_else(|| {
                            AppError::new(
                                "transcribing",
                                "malformed",
                                "Provider returned an invalid upload reference.",
                            )
                        })?;
                    let mut submission = json!({"audio_url":url,"speech_models":[t.model],"language_code":"en","speaker_labels":false,"punctuate":true,"format_text":true});
                    if !t.prompt.trim().is_empty() {
                        submission["prompt"] = json!(t.prompt);
                    }
                    let request = self
                        .client
                        .post("https://api.assemblyai.com/v2/transcript")
                        .header("authorization", &key)
                        .json(&submission);
                    let submitted = self.json(request, "transcribing").await?;
                    let id = submitted
                        .get("id")
                        .and_then(Value::as_str)
                        .filter(|s| {
                            s.len() <= 150
                                && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
                        })
                        .ok_or_else(|| {
                            AppError::new(
                                "transcribing",
                                "malformed",
                                "Provider returned an invalid job identifier.",
                            )
                        })?
                        .to_owned();
                    remote(id.clone())?;
                    id
                };
                if remote_id.is_empty()
                    || !remote_id
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '-')
                {
                    return Err(AppError::input("Invalid saved remote job identifier."));
                }
                let deadline = tokio::time::Instant::now() + Duration::from_secs(1800);
                let mut failures = 0u32;
                let mut delay = 2;
                loop {
                    if tokio::time::Instant::now() >= deadline {
                        return Err(AppError::new(
                            "transcribing",
                            "poll_timeout",
                            "Remote transcription is still pending. Resume it from history later.",
                        ));
                    }
                    let result = self
                        .json(
                            self.client
                                .get(format!(
                                    "https://api.assemblyai.com/v2/transcript/{remote_id}"
                                ))
                                .header("authorization", &key)
                                .timeout(Duration::from_secs(60)),
                            "transcribing",
                        )
                        .await;
                    match result {
                        Ok(v) => {
                            failures = 0;
                            match v["status"].as_str(){
Some("completed")=>break v,Some("error")=>return Err(AppError::new("transcribing","remote_failure","Remote transcription failed. Review the source audio and retry.")),Some("queued"|"processing")=>{
}
,_=>return Err(AppError::new("transcribing","malformed","Remote job status is invalid."))}
                        }
                        Err(e) => {
                            if !["transport", "timeout", "rate_limit", "provider_unavailable"]
                                .contains(&e.category.as_str())
                                || failures >= 3
                            {
                                return Err(e);
                            }
                            failures += 1;
                        }
                    }
                    tokio::time::sleep(Duration::from_secs(if failures > 0 { 60 } else { delay }))
                        .await;
                    delay = (delay * 2).min(15);
                }
            }
            _ => return Err(AppError::input("Unsupported transcription provider.")),
        };
        extract_transcript(&value, &t.provider)
    }
}
pub fn extract_transcript(v: &Value, provider: &str) -> Result<String> {
    let text = if provider == "deepgram" {
        v.pointer("/results/channels/0/alternatives/0/transcript")
    } else {
        v.get("text")
    };
    let text = text.and_then(Value::as_str).ok_or_else(|| {
        AppError::new(
            "transcribing",
            "malformed",
            "Provider returned no valid transcript field.",
        )
    })?;
    if text.trim().is_empty() {
        return Err(AppError::new(
            "transcribing",
            "no_speech",
            "No speech detected. Original audio is retained.",
        ));
    }
    Ok(text.to_owned())
}
pub fn transform_request(input: &str, p: &Preset) -> Result<Value> {
    let user = p.render(input)?;
    let m = model(&p.provider, &p.model, "transformation")?;
    // UTF-8 byte count is a conservative token upper bound; do not silently shorten input.
    let input_budget = (p.system.len() + user.len()) as u32;
    let effort = p.reasoning.as_deref().unwrap_or(m.default_reasoning);
    let reserve: u32 = match effort {
        "none" | "off" => 0,
        "high" | "xhigh" | "max" => 32_768,
        _ => 8192,
    };
    let output_budget = (input.len() as u32)
        .saturating_add(1024)
        .max(4096)
        .saturating_add(reserve);
    if output_budget > m.output
        || input_budget > m.context.saturating_sub(m.output)
        || input_budget.saturating_add(output_budget) > m.context
    {
        return Err(AppError::new("transforming","context_limit","Text is too large for this model's conservative input/output budget. Select a model with larger limits."));
    }
    let mut v = if p.provider == "openai" {
        json!({
        "model":p.model,"instructions":p.system,"input":user,"max_output_tokens":output_budget,"reasoning":{
        "effort":effort}
        ,"store":false}
        )
    } else {
        json!({
        "model":p.model,"system":p.system,"messages":[{
        "role":"user","content":user}
        ],"max_tokens":output_budget}
        )
    };
    if p.provider == "anthropic" {
        v["thinking"] = if effort == "off" {
            json!({"type":if p.model == "claude-sonnet-5-5" {"between_tools"} else {"disabled"}})
        } else {
            json!({"type":"adaptive"})
        };
        v["output_config"] = json!({"effort":if effort=="off" {"low"} else {effort}});
    }
    if let Some(t) = p.temperature {
        v["temperature"] = json!(t);
    }
    Ok(v)
}
#[async_trait]
impl TransformationProvider for Providers {
    async fn transform(&self, input: &str, p: &Preset) -> Result<String> {
        let body = transform_request(input, p)?;
        let key = self.keys.get(&p.provider)?;
        let req = if p.provider == "openai" {
            self.client
                .post("https://api.openai.com/v1/responses")
                .bearer_auth(key)
        } else {
            self.client
                .post("https://api.anthropic.com/v1/messages")
                .header("x-api-key", key)
                .header("anthropic-version", "2023-06-01")
        };
        let v = self
            .json(
                req.timeout(Duration::from_secs(600)).json(&body),
                "transforming",
            )
            .await?;
        extract_transform(&v, &p.provider)
    }
}
pub fn extract_transform(v: &Value, provider: &str) -> Result<String> {
    let mut text = String::new();
    let bad = || {
        AppError::new("transforming","incomplete_output","Provider output was incomplete, refused, or malformed. Previously saved text is preserved.")
    };
    if provider == "openai" {
        if v["status"] != "completed" || !v["incomplete_details"].is_null() {
            return Err(bad());
        }
        let items = v["output"].as_array().ok_or_else(bad)?;
        for item in items {
            if item["type"] == "message" {
                if item["status"].as_str().is_some_and(|s| s != "completed") {
                    return Err(bad());
                }
                for c in item["content"].as_array().ok_or_else(bad)? {
                    if c["type"] != "output_text" {
                        return Err(bad());
                    }
                    text.push_str(c["text"].as_str().ok_or_else(bad)?);
                }
            } else if item["type"] != "reasoning" {
                return Err(bad());
            }
        }
    } else {
        if v["stop_reason"] != "end_turn" {
            return Err(bad());
        }
        for c in v["content"].as_array().ok_or_else(bad)? {
            if c["type"] == "thinking" || c["type"] == "redacted_thinking" {
                continue;
            }
            if c["type"] != "text" {
                return Err(bad());
            }
            text.push_str(c["text"].as_str().ok_or_else(bad)?);
        }
    }
    if text.trim().is_empty() {
        return Err(bad());
    }
    Ok(text)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn refuses_partial_or_empty() {
        for v in [
            json!({
            "status":"incomplete","output":[]}
            ),
            json!({
            "status":"completed","output":[{
            "type":"message","content":[{
            "type":"refusal"}
            ]}
            ]}
            ),
            json!({
            "status":"completed","output":[]}
            ),
        ] {
            assert!(extract_transform(&v, "openai").is_err());
        }
        assert!(extract_transform(
            &json!({
            "stop_reason":"max_tokens","content":[{
            "type":"text","text":"cut"}
            ]}
            ),
            "anthropic"
        )
        .is_err());
    }
    #[test]
    fn preserves_plain_text() {
        assert_eq!(
            extract_transform(
                &json!({
                "status":"completed","output":[{
                "type":"message","status":"completed","content":[{
                "type":"output_text","text":"<script> **literal**"}
                ]}
                ]}
                ),
                "openai"
            )
            .unwrap(),
            "<script> **literal**"
        );
    }
    #[test]
    fn thinking_serialization_and_legacy_presets_obey_model_capabilities() {
        let mut legacy = serde_json::to_value(Preset::seed()).unwrap();
        legacy.as_object_mut().unwrap().remove("reasoning");
        let old: Preset = serde_json::from_value(legacy).unwrap();
        assert_eq!(
            transform_request("hello", &old).unwrap()["reasoning"]["effort"],
            "none"
        );
        for model in crate::domain::catalog()
            .into_iter()
            .filter(|m| m.stage == "transformation")
        {
            for effort in &model.reasoning {
                let mut preset = Preset::seed();
                preset.provider = model.provider.into();
                preset.model = model.id.into();
                preset.reasoning = Some((*effort).into());
                let request = transform_request("A short transcript.", &preset).unwrap();
                if model.provider == "openai" {
                    assert_eq!(request["reasoning"]["effort"], *effort);
                    assert_eq!(request["store"], false);
                } else {
                    let kind = if *effort == "off" {
                        if model.id == "claude-sonnet-5-5" {
                            "between_tools"
                        } else {
                            "disabled"
                        }
                    } else {
                        "adaptive"
                    };
                    assert_eq!(request["thinking"]["type"], kind);
                    assert_eq!(
                        request["output_config"]["effort"],
                        if *effort == "off" { "low" } else { effort }
                    );
                }
                let budget = request[if model.provider == "openai" {
                    "max_output_tokens"
                } else {
                    "max_tokens"
                }]
                .as_u64()
                .unwrap();
                assert!(budget <= u64::from(model.output));
                if !matches!(*effort, "none" | "off") {
                    assert!(budget > 8192);
                }
            }
        }
        let mut preset = Preset::seed();
        preset.model = "gpt-6.1-sol".into();
        preset.reasoning = Some("none".into());
        assert!(preset.validate().is_err());
        preset.model = "gpt-6-luna".into();
        preset.reasoning = Some("high".into());
        preset.temperature = Some(1.);
        assert!(preset.validate().is_err());
        preset.reasoning = Some("none".into());
        assert!(preset.validate().is_ok());
    }
    #[test]
    fn model_parameters() {
        let p = Preset::seed();
        let v = transform_request("hello", &p).unwrap();
        assert!(v.get("temperature").is_none());
        assert_eq!(v["reasoning"]["effort"], "none");
        assert!(transform_request(&"a".repeat(130_000), &p).is_err());
    }
}

#[cfg(test)]
mod transport_tests {
    use super::*;
    struct Keys;
    impl CredentialStore for Keys {
        fn get(&self, _: &str) -> Result<String> {
            Ok("test-key".into())
        }
        fn set(&self, _: &str, _: &str) -> Result<()> {
            Ok(())
        }
        fn delete(&self, _: &str) -> Result<()> {
            Ok(())
        }
    }
    async fn server(status: u16, body: &'static str) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = [0; 4096];
            let _ = socket.read(&mut bytes).await;
            let response=format!("HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\nContent-Type: application/json\r\n\r\n{body}",body.len());
            socket.write_all(response.as_bytes()).await.unwrap();
        });
        format!("http://{address}")
    }
    #[tokio::test]
    async fn categorizes_http_errors_without_echoing_body_or_credentials() {
        let p = Providers::new(Arc::new(Keys)).unwrap();
        for (code, category) in [
            (401, "authentication"),
            (403, "authentication"),
            (413, "size"),
            (429, "rate_limit"),
            (500, "provider_unavailable"),
            (404, "unsupported_request"),
        ] {
            let url = server(code, "secret-key private transcript").await;
            let e = p.json(p.client.get(url), "transcribing").await.unwrap_err();
            assert_eq!(e.category, category);
            assert!(!e.message.contains("secret-key"));
            assert!(!e.message.contains("private transcript"));
        }
    }
    #[tokio::test]
    async fn rejects_malformed_json_and_accepts_structured_success() {
        let p = Providers::new(Arc::new(Keys)).unwrap();
        let url = server(200, "not-json").await;
        assert_eq!(
            p.json(p.client.get(url), "transforming")
                .await
                .unwrap_err()
                .category,
            "malformed"
        );
        let url = server(200, "{\"text\":\"speech\"}").await;
        let v = p.json(p.client.get(url), "transcribing").await.unwrap();
        assert_eq!(extract_transcript(&v, "openai").unwrap(), "speech");
    }
}
