//! # Groq API Client & Fallback Provider
//!
//! Provides ultra-fast inference and automatic fallback when Gemini is rate-limited or unavailable.
//! Uses OpenAI-compatible endpoints with models:
//! - llama-3.3-70b-versatile (Primary)
//! - llama-3.1-8b-instant (Fast fallback)
//! - deepseek-r1-distill-llama-70b (Reasoning)
//! - llama-3.2-11b-vision-preview (Multimodal screen/image)

use crate::{
    ai::provider::{
        CompletionRequest, CompletionResponse, MessageRole, MultimodalRequest,
        TokenUsage, UNTRUSTED_CONTEXT_NOTICE,
    },
    error::OpenMateError,
};
use base64::Engine;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

pub fn get_default_groq_api_key() -> String {
    if let Ok(key) = crate::platform::keychain::get_api_key() {
        if key.starts_with("gsk_") {
            return key;
        }
    }
    std::env::var("GROQ_API_KEY").unwrap_or_default()
}
pub const GROQ_BASE_URL: &str = "https://api.groq.com/openai/v1/chat/completions";

pub const GROQ_MODELS: &[&str] = &[
    "openai/gpt-oss-120b",
    "openai/gpt-oss-20b",
    "qwen/qwen3.6-27b",
    "groq/compound",
    "groq/compound-mini",
];

pub const GROQ_VISION_MODEL: &str = "qwen/qwen3.6-27b";

fn strip_think_tags(text: &str) -> String {
    if let Some(end_idx) = text.find("</think>") {
        text[end_idx + 8..].trim().to_string()
    } else {
        text.trim().to_string()
    }
}

pub struct GroqClient {
    client: reqwest::Client,
}

impl GroqClient {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(45))
                .build()
                .unwrap_or_default(),
        }
    }

    /// Generate text completion with automatic multi-model retry on Groq.
    pub async fn generate_text(
        &self,
        request: CompletionRequest,
        api_key_override: Option<&str>,
    ) -> Result<CompletionResponse, OpenMateError> {
        let fallback_key = get_default_groq_api_key();
        let api_key = api_key_override.unwrap_or(&fallback_key);

        // Build OpenAI-format messages
        let mut messages = Vec::new();

        // 1. System prompt + Context
        let mut system_text = request.system_instruction;
        if let Some(ctx) = request.context {
            if !ctx.trim().is_empty() {
                let formatted = if ctx.contains(UNTRUSTED_CONTEXT_NOTICE) {
                    ctx
                } else {
                    format!("{}\n\nDesktop Context:\n{}", UNTRUSTED_CONTEXT_NOTICE, ctx)
                };
                if system_text.is_empty() {
                    system_text = formatted;
                } else {
                    system_text = format!("{}\n\n{}", system_text, formatted);
                }
            }
        }

        if !system_text.is_empty() {
            messages.push(OpenAiMessage {
                role: "system".to_string(),
                content: OpenAiMessageContent::Text(system_text),
            });
        }

        // 2. Conversation messages
        for msg in request.messages {
            let role = match msg.role {
                MessageRole::User => "user",
                MessageRole::Assistant => "assistant",
                MessageRole::System => "system",
            };
            messages.push(OpenAiMessage {
                role: role.to_string(),
                content: OpenAiMessageContent::Text(msg.content),
            });
        }

        // Try models in cascade order
        let mut last_err = OpenMateError::ProviderError("No Groq models available".to_string());
        for model in GROQ_MODELS {
            info!("Attempting Groq completion with model: {}", model);
            let req_body = OpenAiChatRequest {
                model: model.to_string(),
                messages: messages.clone(),
                temperature: Some(0.7),
                max_tokens: Some(2048),
            };

            let res = self
                .client
                .post(GROQ_BASE_URL)
                .bearer_auth(api_key)
                .json(&req_body)
                .send()
                .await;

            match res {
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_success() {
                        let parsed: OpenAiChatResponse = resp
                            .json()
                            .await
                            .map_err(|e| OpenMateError::SerializationError(e.to_string()))?;

                        if let Some(choice) = parsed.choices.into_iter().next() {
                            let usage = parsed.usage.map(|u| TokenUsage {
                                prompt_tokens: u.prompt_tokens,
                                completion_tokens: u.completion_tokens,
                            });
                            info!("Groq ({}) responded successfully", model);
                            let raw_content = choice.message.content.unwrap_or_default();
                            return Ok(CompletionResponse {
                                content: strip_think_tags(&raw_content),
                                tool_call: None,
                                usage,
                            });
                        }
                    } else if status == StatusCode::TOO_MANY_REQUESTS {
                        warn!("Groq model {} rate limited (429), trying next model...", model);
                        last_err = OpenMateError::RateLimited;
                        continue;
                    } else {
                        let text = resp.text().await.unwrap_or_default();
                        warn!("Groq model {} error HTTP {}: {}", model, status, text);
                        last_err = OpenMateError::AiProviderError(status.as_u16(), text);
                    }
                }
                Err(e) => {
                    warn!("Network error calling Groq ({}): {}", model, e);
                    last_err = OpenMateError::NetworkError(e.to_string());
                }
            }
        }

        Err(last_err)
    }

    /// Multimodal image + text analysis using Groq's vision model.
    pub async fn analyze_image_with_text(
        &self,
        request: MultimodalRequest,
        api_key_override: Option<&str>,
    ) -> Result<CompletionResponse, OpenMateError> {
        let fallback_key = get_default_groq_api_key();
        let api_key = api_key_override.unwrap_or(&fallback_key);
        let b64_image = base64::engine::general_purpose::STANDARD.encode(&request.image_bytes);
        let data_url = format!("data:{};base64,{}", request.image_mime_type, b64_image);

        let mut messages = Vec::new();

        let mut system_text = request.system_instruction;
        if let Some(ctx) = request.context {
            if !ctx.trim().is_empty() {
                let formatted = if ctx.contains(UNTRUSTED_CONTEXT_NOTICE) {
                    ctx
                } else {
                    format!("{}\n\nDesktop Context:\n{}", UNTRUSTED_CONTEXT_NOTICE, ctx)
                };
                if system_text.is_empty() {
                    system_text = formatted;
                } else {
                    system_text = format!("{}\n\n{}", system_text, formatted);
                }
            }
        }

        if !system_text.is_empty() {
            messages.push(OpenAiMessage {
                role: "system".to_string(),
                content: OpenAiMessageContent::Text(system_text),
            });
        }

        let num_msgs = request.messages.len();
        for (idx, msg) in request.messages.into_iter().enumerate() {
            let role = match msg.role {
                MessageRole::User => "user",
                MessageRole::Assistant => "assistant",
                MessageRole::System => "system",
            };

            // Attach image to the last turn
            if idx == num_msgs.saturating_sub(1) {
                let parts = vec![
                    OpenAiContentPart::Text { text: msg.content },
                    OpenAiContentPart::ImageUrl {
                        image_url: OpenAiImageUrl { url: data_url.clone() },
                    },
                ];
                messages.push(OpenAiMessage {
                    role: role.to_string(),
                    content: OpenAiMessageContent::Parts(parts),
                });
            } else {
                messages.push(OpenAiMessage {
                    role: role.to_string(),
                    content: OpenAiMessageContent::Text(msg.content),
                });
            }
        }

        if messages.is_empty() {
            messages.push(OpenAiMessage {
                role: "user".to_string(),
                content: OpenAiMessageContent::Parts(vec![
                    OpenAiContentPart::Text {
                        text: "Analyze this image".to_string(),
                    },
                    OpenAiContentPart::ImageUrl {
                        image_url: OpenAiImageUrl { url: data_url },
                    },
                ]),
            });
        }

        info!("Sending multimodal request to Groq vision model: {}", GROQ_VISION_MODEL);

        let req_body = OpenAiChatRequest {
            model: GROQ_VISION_MODEL.to_string(),
            messages,
            temperature: Some(0.7),
            max_tokens: Some(2048),
        };

        let res = self
            .client
            .post(GROQ_BASE_URL)
            .bearer_auth(api_key)
            .json(&req_body)
            .send()
            .await
            .map_err(|e| OpenMateError::NetworkError(e.to_string()))?;

        let status = res.status();
        if !status.is_success() {
            let err_text = res.text().await.unwrap_or_default();
            return Err(OpenMateError::AiProviderError(status.as_u16(), err_text));
        }

        let parsed: OpenAiChatResponse = res
            .json()
            .await
            .map_err(|e| OpenMateError::SerializationError(e.to_string()))?;

        let choice = parsed
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| OpenMateError::ProviderError("No response candidates from Groq vision".to_string()))?;

        let usage = parsed.usage.map(|u| TokenUsage {
            prompt_tokens: u.prompt_tokens,
            completion_tokens: u.completion_tokens,
        });

        Ok(CompletionResponse {
            content: choice.message.content.unwrap_or_default(),
            tool_call: None,
            usage,
        })
    }
}

// ── OpenAI compatible DTOs for Groq ─────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
struct OpenAiChatRequest {
    model: String,
    messages: Vec<OpenAiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
struct OpenAiMessage {
    role: String,
    content: OpenAiMessageContent,
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
enum OpenAiMessageContent {
    Text(String),
    Parts(Vec<OpenAiContentPart>),
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
enum OpenAiContentPart {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image_url")]
    ImageUrl { image_url: OpenAiImageUrl },
}

#[derive(Debug, Clone, Serialize)]
struct OpenAiImageUrl {
    url: String,
}

#[derive(Debug, Deserialize)]
struct OpenAiChatResponse {
    choices: Vec<OpenAiChoice>,
    usage: Option<OpenAiUsage>,
}

#[derive(Debug, Deserialize)]
struct OpenAiChoice {
    message: OpenAiResponseMessage,
}

#[derive(Debug, Deserialize)]
struct OpenAiResponseMessage {
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAiUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
}
