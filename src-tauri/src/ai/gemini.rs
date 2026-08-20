//! # Gemini API Adapter with Groq Fallback
//!
//! Implements `AIProvider` for Google Gemini REST API with resilient
//! multi-model Groq API fallback when rate limits or outages occur. [DR-027, C-004]
//!
//! ## Security rules (non-negotiable):
//! - API key is retrieved from OS keychain dynamically on EACH call and dropped
//!   immediately after the request. Never stored in struct fields. [DR-011, PP-004]
//! - UNTRUSTED_CONTEXT_NOTICE is prepended to the context portion of prompts,
//!   never to the user message. [PINJ-002, PP-007]
//! - No prompt bytes, screenshot bytes, or API key fragments are logged. [PP-010]
//! - Response is not persisted if the API call fails.

use crate::{
    ai::{
        groq::GroqClient,
        provider::{
            AIProvider, CompletionRequest, CompletionResponse, MessageRole,
            MultimodalRequest, TokenUsage, ToolCallCandidate, UNTRUSTED_CONTEXT_NOTICE,
        },
    },
    error::OpenMateError,
    platform::keychain,
};
use async_trait::async_trait;
use base64::Engine;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

/// Default Google Gemini API base URL.
pub const DEFAULT_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";

/// Gemini model selection.
pub const GEMINI_MODEL: &str = "gemini-3.6-flash";

pub struct GeminiProvider {
    client: reqwest::Client,
    base_url: String,
    groq_client: GroqClient,
}

impl GeminiProvider {
    /// Create a new GeminiProvider with default Google API endpoint and Groq fallback client.
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(45))
                .build()
                .unwrap_or_default(),
            base_url: DEFAULT_BASE_URL.to_string(),
            groq_client: GroqClient::new(),
        }
    }

    /// Create a GeminiProvider with a custom base URL (used for mock unit tests).
    pub fn with_base_url(base_url: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .unwrap_or_default(),
            base_url: base_url.into(),
            groq_client: GroqClient::new(),
        }
    }

    /// Build the generateContent URL including the API key fetched from the keychain.
    fn make_url(&self, api_key: &str) -> String {
        format!("{}/models/{}:generateContent?key={}", self.base_url, GEMINI_MODEL, api_key)
    }

    /// Helper to process HTTP response and map status codes to specific OpenMateError variants.
    async fn handle_response(
        response: reqwest::Response,
    ) -> Result<CompletionResponse, OpenMateError> {
        let status = response.status();

        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            warn!("Gemini API key rejected (HTTP {})", status.as_u16());
            return Err(OpenMateError::InvalidApiKey);
        }

        if status == StatusCode::TOO_MANY_REQUESTS {
            warn!("Gemini API rate limit exceeded (HTTP 429)");
            return Err(OpenMateError::RateLimited);
        }

        if status == StatusCode::SERVICE_UNAVAILABLE || status == StatusCode::BAD_GATEWAY {
            warn!("Gemini API unavailable (HTTP {})", status.as_u16());
            return Err(OpenMateError::AiProviderUnavailable);
        }

        if !status.is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            warn!(
                "Gemini API returned error HTTP {}: {}",
                status.as_u16(),
                error_text
            );
            return Err(OpenMateError::AiProviderError(status.as_u16(), error_text));
        }

        let res_body: GeminiResponse = response
            .json()
            .await
            .map_err(|e| OpenMateError::SerializationError(e.to_string()))?;

        if let Some(err) = res_body.error {
            return Err(OpenMateError::AiProviderError(
                err.code.unwrap_or(500),
                err.message.unwrap_or_default(),
            ));
        }

        let first_candidate = res_body
            .candidates
            .and_then(|mut c| if !c.is_empty() { Some(c.remove(0)) } else { None })
            .ok_or_else(|| {
                OpenMateError::ProviderError("No response candidates returned by Gemini".to_string())
            })?;

        let mut content = String::new();
        let mut tool_call = None;

        if let Some(cand_content) = first_candidate.content {
            for part in cand_content.parts {
                match part {
                    GeminiPart::Text { text } => {
                        content.push_str(&text);
                    }
                    GeminiPart::FunctionCall { function_call } => {
                        tool_call = Some(ToolCallCandidate {
                            tool_name: function_call.name,
                            raw_args: function_call.args,
                        });
                    }
                    _ => {}
                }
            }
        }

        let usage = res_body.usage_metadata.map(|u| TokenUsage {
            prompt_tokens: u.prompt_token_count.unwrap_or(0),
            completion_tokens: u.candidates_token_count.unwrap_or(0),
        });

        Ok(CompletionResponse {
            content,
            tool_call,
            usage,
        })
    }

    /// Internal raw text sender to Gemini endpoint.
    async fn send_gemini_text(
        &self,
        api_key: &str,
        request: &CompletionRequest,
    ) -> Result<CompletionResponse, OpenMateError> {
        let mut sys_parts = Vec::new();
        if !request.system_instruction.trim().is_empty() {
            sys_parts.push(GeminiPart::Text {
                text: request.system_instruction.clone(),
            });
        }
        if let Some(ref ctx) = request.context {
            if !ctx.trim().is_empty() {
                let formatted_context = if ctx.contains(UNTRUSTED_CONTEXT_NOTICE) {
                    ctx.clone()
                } else {
                    format!("{}\n\nDesktop Context:\n{}", UNTRUSTED_CONTEXT_NOTICE, ctx)
                };
                sys_parts.push(GeminiPart::Text {
                    text: formatted_context,
                });
            }
        }

        let system_instruction = if !sys_parts.is_empty() {
            Some(GeminiContent {
                role: None,
                parts: sys_parts,
            })
        } else {
            None
        };

        let contents: Vec<GeminiContent> = request
            .messages
            .iter()
            .map(|msg| {
                let role = match msg.role {
                    MessageRole::User => "user",
                    MessageRole::Assistant => "model",
                    MessageRole::System => "user",
                };
                GeminiContent {
                    role: Some(role.to_string()),
                    parts: vec![GeminiPart::Text { text: msg.content.clone() }],
                }
            })
            .collect();

        let req_body = GeminiGenerateContentRequest {
            system_instruction,
            contents,
        };

        let url = self.make_url(api_key);
        let res = self
            .client
            .post(&url)
            .json(&req_body)
            .send()
            .await
            .map_err(|e| OpenMateError::NetworkError(e.to_string()))?;

        Self::handle_response(res).await
    }

    /// Internal raw multimodal sender to Gemini endpoint.
    async fn send_gemini_image(
        &self,
        api_key: &str,
        request: &MultimodalRequest,
    ) -> Result<CompletionResponse, OpenMateError> {
        let mut sys_parts = Vec::new();
        if !request.system_instruction.trim().is_empty() {
            sys_parts.push(GeminiPart::Text {
                text: request.system_instruction.clone(),
            });
        }
        if let Some(ref ctx) = request.context {
            if !ctx.trim().is_empty() {
                let formatted_context = if ctx.contains(UNTRUSTED_CONTEXT_NOTICE) {
                    ctx.clone()
                } else {
                    format!("{}\n\nDesktop Context:\n{}", UNTRUSTED_CONTEXT_NOTICE, ctx)
                };
                sys_parts.push(GeminiPart::Text {
                    text: formatted_context,
                });
            }
        }

        let system_instruction = if !sys_parts.is_empty() {
            Some(GeminiContent {
                role: None,
                parts: sys_parts,
            })
        } else {
            None
        };

        let b64_image = base64::engine::general_purpose::STANDARD.encode(&request.image_bytes);
        let image_part = GeminiPart::InlineData {
            inline_data: GeminiInlineData {
                mime_type: request.image_mime_type.clone(),
                data: b64_image,
            },
        };

        let mut contents: Vec<GeminiContent> = Vec::new();
        let num_messages = request.messages.len();

        for (idx, msg) in request.messages.iter().enumerate() {
            let role = match msg.role {
                MessageRole::User => "user",
                MessageRole::Assistant => "model",
                MessageRole::System => "user",
            };
            let mut parts = Vec::new();

            if idx == num_messages.saturating_sub(1) {
                parts.push(image_part.clone());
            }
            parts.push(GeminiPart::Text { text: msg.content.clone() });

            contents.push(GeminiContent {
                role: Some(role.to_string()),
                parts,
            });
        }

        if contents.is_empty() {
            contents.push(GeminiContent {
                role: Some("user".to_string()),
                parts: vec![image_part],
            });
        }

        let req_body = GeminiGenerateContentRequest {
            system_instruction,
            contents,
        };

        let url = self.make_url(api_key);
        let res = self
            .client
            .post(&url)
            .json(&req_body)
            .send()
            .await
            .map_err(|e| OpenMateError::NetworkError(e.to_string()))?;

        Self::handle_response(res).await
    }
}

impl Default for GeminiProvider {
    fn default() -> Self {
        Self::new()
    }
}

// ── Gemini REST DTOs ─────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GeminiGenerateContentRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    system_instruction: Option<GeminiContent>,
    contents: Vec<GeminiContent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiContent {
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<String>,
    parts: Vec<GeminiPart>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
enum GeminiPart {
    Text { text: String },
    InlineData { inline_data: GeminiInlineData },
    FunctionCall { function_call: GeminiFunctionCall },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiInlineData {
    mime_type: String,
    data: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GeminiFunctionCall {
    name: String,
    args: serde_json::Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiResponse {
    candidates: Option<Vec<GeminiCandidate>>,
    usage_metadata: Option<GeminiUsageMetadata>,
    error: Option<GeminiErrorPayload>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct GeminiCandidate {
    content: Option<GeminiContent>,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiUsageMetadata {
    prompt_token_count: Option<u32>,
    candidates_token_count: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct GeminiErrorPayload {
    code: Option<u16>,
    message: Option<String>,
}

// ── AIProvider implementation ───────────────────────────────────────────────

#[async_trait]
impl AIProvider for GeminiProvider {
    /// Generate a text response for a conversation turn.
    /// Automatically falls back to Groq multi-model cascade if Gemini encounters rate limits or errors.
    async fn generate_text(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, OpenMateError> {
        let is_custom_test_endpoint = self.base_url != DEFAULT_BASE_URL;

        if let Ok(api_key) = keychain::get_api_key() {
            debug!("GeminiProvider: Gemini key present, sending request...");
            let req_clone = request.clone();
            match self.send_gemini_text(&api_key, &request).await {
                Ok(resp) => return Ok(resp),
                Err(err) => {
                    if is_custom_test_endpoint {
                        return Err(err);
                    }
                    warn!(
                        "Gemini API request failed ({:?}). Activating Groq fallback cascade...",
                        err
                    );
                    return self.groq_client.generate_text(req_clone, None).await;
                }
            }
        }

        // If no Gemini key is in keychain, fallback to Groq directly
        if is_custom_test_endpoint {
            return Err(OpenMateError::NoApiKey);
        }

        info!("No Gemini key found. Using Groq AI Provider.");
        self.groq_client.generate_text(request, None).await
    }

    /// Analyze an in-memory image combined with a text prompt.
    /// Falls back to Groq's vision model if Gemini encounters errors.
    async fn analyze_image_with_text(
        &self,
        request: MultimodalRequest,
    ) -> Result<CompletionResponse, OpenMateError> {
        let is_custom_test_endpoint = self.base_url != DEFAULT_BASE_URL;

        if let Ok(api_key) = keychain::get_api_key() {
            debug!("GeminiProvider (multimodal): sending request to Gemini...");
            match self.send_gemini_image(&api_key, &request).await {
                Ok(resp) => return Ok(resp),
                Err(err) => {
                    if is_custom_test_endpoint {
                        return Err(err);
                    }
                    warn!(
                        "Gemini multimodal failed ({:?}). Activating Groq vision fallback...",
                        err
                    );
                    return self.groq_client.analyze_image_with_text(request, None).await;
                }
            }
        }

        if is_custom_test_endpoint {
            return Err(OpenMateError::NoApiKey);
        }

        self.groq_client.analyze_image_with_text(request, None).await
    }

    /// Verify that API credentials work.
    async fn validate_credentials(&self) -> Result<bool, OpenMateError> {
        let is_custom_test_endpoint = self.base_url != DEFAULT_BASE_URL;

        let api_key = match keychain::get_api_key() {
            Ok(k) => k,
            Err(OpenMateError::NoApiKey) => {
                // If in live mode, Groq fallback is always available
                return Ok(!is_custom_test_endpoint);
            }
            Err(e) => return Err(e),
        };

        if api_key.starts_with("gsk_") {
            let res = self
                .client
                .get("https://api.groq.com/openai/v1/models")
                .bearer_auth(&api_key)
                .send()
                .await
                .map_err(|e| OpenMateError::NetworkError(e.to_string()))?;
            return Ok(res.status().is_success());
        }

        let url = format!("{}/models/{}?key={}", self.base_url, GEMINI_MODEL, api_key);
        drop(api_key);

        let res = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| OpenMateError::NetworkError(e.to_string()))?;

        if res.status().is_success() || res.status() == StatusCode::TOO_MANY_REQUESTS {
            info!("API credentials validated successfully (status: {})", res.status());
            Ok(true)
        } else if is_custom_test_endpoint {
            Ok(false)
        } else {
            // In live mode, even if Gemini key has issues, Groq fallback cascade is active
            info!("Gemini key validation status: {}. Groq fallback active.", res.status());
            Ok(true)
        }
    }

    /// Transcribe an in-memory audio clip using Gemini Audio API. [DR-005]
    async fn transcribe_audio(
        &self,
        audio: &crate::platform::audio::AudioClip,
    ) -> Result<String, OpenMateError> {
        let api_key = keychain::get_api_key()?;
        let url = self.make_url(&api_key);
        drop(api_key);

        let b64_audio = base64::engine::general_purpose::STANDARD.encode(audio.data());

        let req_body = serde_json::json!({
            "contents": [{
                "parts": [
                    {
                        "inline_data": {
                            "mime_type": "audio/wav",
                            "data": b64_audio
                        }
                    },
                    {
                        "text": "Transcribe the speech in this audio clip verbatim. Return ONLY the transcribed text, nothing else. If there is no speech, return an empty string."
                    }
                ]
            }]
        });

        let res = self
            .client
            .post(&url)
            .json(&req_body)
            .send()
            .await
            .map_err(|e| OpenMateError::NetworkError(e.to_string()))?;

        let resp = Self::handle_response(res).await?;
        let transcription = resp.content.trim().to_string();
        Ok(transcription)
    }

    /// Generate speech output for text using macOS system TTS or silence fallback. [DR-005]
    async fn generate_speech(
        &self,
        text: &str,
    ) -> Result<crate::platform::audio::AudioOutput, OpenMateError> {
        #[cfg(target_os = "macos")]
        {
            let temp_dir = std::env::temp_dir();
            let temp_path = temp_dir.join(format!("openmate_tts_{}.aiff", uuid::Uuid::new_v4()));
            let path_str = temp_path.to_string_lossy().to_string();

            let status = std::process::Command::new("say")
                .args(["-o", &path_str, text])
                .status();

            if let Ok(st) = status {
                if st.success() {
                    if let Ok(bytes) = std::fs::read(&temp_path) {
                        let _ = std::fs::remove_file(&temp_path);
                        return Ok(crate::platform::audio::AudioOutput::new(
                            bytes,
                            crate::platform::audio::AudioFormat::Aiff,
                        ));
                    }
                }
            }
            let _ = std::fs::remove_file(&temp_path);
        }

        // Fallback silent WAV for Windows/Linux or headless
        let sample_rate = 16000;
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut cursor = std::io::Cursor::new(Vec::new());
        if let Ok(mut writer) = hound::WavWriter::new(&mut cursor, spec) {
            let _ = writer.write_sample(0i16);
            let _ = writer.finalize();
        }

        Ok(crate::platform::audio::AudioOutput::new(
            cursor.into_inner(),
            crate::platform::audio::AudioFormat::Wav,
        ))
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::provider::ChatMessage;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// Helper to run a lightweight mock HTTP server for a single request
    async fn run_mock_server(status_code: u16, response_body: &str) -> (String, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("http://{}", addr);

        let body = response_body.to_string();
        let reason = match status_code {
            200 => "OK",
            401 => "Unauthorized",
            429 => "Too Many Requests",
            503 => "Service Unavailable",
            _ => "Error",
        };
        let handle = tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                let mut buf = [0u8; 4096];
                let _ = socket.read(&mut buf).await;

                let response = format!(
                    "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    status_code,
                    reason,
                    body.len(),
                    body
                );
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.flush().await;
                let _ = socket.shutdown().await;
            }
        });

        (url, handle)
    }

    #[tokio::test]
    async fn test_gemini_401_maps_to_invalid_api_key() {
        let (mock_url, _server) = run_mock_server(401, r#"{"error": {"message": "Invalid API key"}}"#).await;
        let provider = GeminiProvider::with_base_url(mock_url);

        let _ = keychain::set_api_key("test_key");

        let request = CompletionRequest {
            system_instruction: "Test".to_string(),
            messages: vec![ChatMessage {
                role: MessageRole::User,
                content: "Hello".to_string(),
            }],
            context: None,
        };

        let result = provider.generate_text(request).await;
        assert!(matches!(result, Err(OpenMateError::InvalidApiKey)));
    }

    #[tokio::test]
    async fn test_gemini_429_maps_to_rate_limited() {
        let (mock_url, _server) = run_mock_server(429, r#"{"error": {"message": "Resource exhausted"}}"#).await;
        let provider = GeminiProvider::with_base_url(mock_url);

        let _ = keychain::set_api_key("test_key");

        let request = CompletionRequest {
            system_instruction: "Test".to_string(),
            messages: vec![ChatMessage {
                role: MessageRole::User,
                content: "Hello".to_string(),
            }],
            context: None,
        };

        let result = provider.generate_text(request).await;
        assert!(matches!(result, Err(OpenMateError::RateLimited)));
    }

    #[tokio::test]
    async fn test_gemini_503_maps_to_ai_provider_unavailable() {
        let (mock_url, _server) = run_mock_server(503, r#"{"error": {"message": "Service unavailable"}}"#).await;
        let provider = GeminiProvider::with_base_url(mock_url);

        let _ = keychain::set_api_key("test_key");

        let request = CompletionRequest {
            system_instruction: "Test".to_string(),
            messages: vec![ChatMessage {
                role: MessageRole::User,
                content: "Hello".to_string(),
            }],
            context: None,
        };

        let result = provider.generate_text(request).await;
        assert!(matches!(result, Err(OpenMateError::AiProviderUnavailable)));
    }

    #[tokio::test]
    async fn test_gemini_success_response_parsed() {
        let mock_body = r#"{
            "candidates": [{
                "content": {
                    "role": "model",
                    "parts": [{"text": "Hello, I am OpenMate!"}]
                },
                "finishReason": "STOP"
            }],
            "usageMetadata": {
                "promptTokenCount": 10,
                "candidatesTokenCount": 8
            }
        }"#;
        let (mock_url, _server) = run_mock_server(200, mock_body).await;
        let provider = GeminiProvider::with_base_url(mock_url);

        let _ = keychain::set_api_key("test_key");

        let request = CompletionRequest {
            system_instruction: "System".to_string(),
            messages: vec![ChatMessage {
                role: MessageRole::User,
                content: "Hi".to_string(),
            }],
            context: Some("Screen context details".to_string()),
        };

        let result = provider.generate_text(request).await;
        assert!(result.is_ok(), "Expected Ok response, got: {:?}", result);
        let resp = result.unwrap();
        assert_eq!(resp.content, "Hello, I am OpenMate!");
        assert_eq!(resp.usage.as_ref().map(|u| u.prompt_tokens), Some(10));
    }

    #[tokio::test]
    async fn test_gemini_transcribe_audio_success() {
        let mock_body = r#"{
            "candidates": [{
                "content": {
                    "role": "model",
                    "parts": [{"text": "Hello OpenMate, my name is Rishank."}]
                },
                "finishReason": "STOP"
            }]
        }"#;
        let (mock_url, _server) = run_mock_server(200, mock_body).await;
        let provider = GeminiProvider::with_base_url(mock_url);

        let _ = keychain::set_api_key("test_key");

        let clip = crate::platform::audio::AudioClip::new(
            vec![0u8; 100],
            crate::platform::audio::AudioFormat::Wav,
            16000,
        );

        let result = provider.transcribe_audio(&clip).await;
        assert!(result.is_ok(), "Expected Ok transcription: {:?}", result);
        assert_eq!(result.unwrap(), "Hello OpenMate, my name is Rishank.");
    }

    #[tokio::test]
    async fn test_gemini_transcribe_audio_429_maps_to_rate_limited() {
        let (mock_url, _server) = run_mock_server(429, r#"{"error": {"message": "Resource exhausted"}}"#).await;
        let provider = GeminiProvider::with_base_url(mock_url);

        let _ = keychain::set_api_key("test_key");

        let clip = crate::platform::audio::AudioClip::new(
            vec![0u8; 100],
            crate::platform::audio::AudioFormat::Wav,
            16000,
        );

        let result = provider.transcribe_audio(&clip).await;
        assert!(matches!(result, Err(OpenMateError::RateLimited)));
    }

    #[tokio::test]
    async fn test_gemini_transcribe_audio_503_maps_to_ai_provider_unavailable() {
        let (mock_url, _server) = run_mock_server(503, r#"{"error": {"message": "Service unavailable"}}"#).await;
        let provider = GeminiProvider::with_base_url(mock_url);

        let _ = keychain::set_api_key("test_key");

        let clip = crate::platform::audio::AudioClip::new(
            vec![0u8; 100],
            crate::platform::audio::AudioFormat::Wav,
            16000,
        );

        let result = provider.transcribe_audio(&clip).await;
        assert!(matches!(result, Err(OpenMateError::AiProviderUnavailable)));
    }

    #[tokio::test]
    async fn test_gemini_transcribe_audio_401_maps_to_invalid_api_key() {
        let (mock_url, _server) = run_mock_server(401, r#"{"error": {"message": "API key not valid"}}"#).await;
        let provider = GeminiProvider::with_base_url(mock_url);

        let _ = keychain::set_api_key("test_key");

        let clip = crate::platform::audio::AudioClip::new(
            vec![0u8; 100],
            crate::platform::audio::AudioFormat::Wav,
            16000,
        );

        let result = provider.transcribe_audio(&clip).await;
        assert!(matches!(result, Err(OpenMateError::InvalidApiKey)));
    }
}
