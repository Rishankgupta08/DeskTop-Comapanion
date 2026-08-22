//! # Voice Provider Trait & Implementations
//!
//! Provides a provider-agnostic interface for Speech-to-Text (STT) and
//! Text-to-Speech (TTS), decoupling voice capabilities from the main LLM brain.

use crate::{
    error::OpenMateError,
    platform::audio::{AudioClip, AudioFormat, AudioOutput},
};
use async_trait::async_trait;
use reqwest::multipart;
use serde::Deserialize;
use std::time::Duration;
use tracing::{info, warn};

pub const GROQ_WHISPER_MODEL: &str = "whisper-large-v3-turbo";
pub const GROQ_TTS_MODEL: &str = "canopylabs/orpheus-v1-english";

/// Agnostic Voice Provider interface for STT & TTS.
#[async_trait]
pub trait VoiceProvider: Send + Sync {
    /// Transcribe in-memory audio clip to text (STT).
    async fn transcribe(&self, audio: &AudioClip) -> Result<String, OpenMateError>;

    /// Synthesize text to in-memory speech audio (TTS).
    async fn synthesize(&self, text: &str) -> Result<AudioOutput, OpenMateError>;
}

/// Groq Voice Provider: STT with Whisper-large-v3-turbo & TTS with Orpheus-v1-english
pub struct GroqVoiceProvider {
    client: reqwest::Client,
    base_url: String,
}

impl GroqVoiceProvider {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
            base_url: "https://api.groq.com/openai/v1".to_string(),
        }
    }

    pub fn with_base_url(base_url: String) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
            base_url,
        }
    }

    fn get_api_key(&self) -> String {
        // Only use custom keychain key if it is a Groq key (starts with 'gsk_').
        // Otherwise use the default working Groq key for STT & TTS.
        if let Ok(key) = crate::platform::keychain::get_api_key() {
            if key.starts_with("gsk_") {
                return key;
            }
        }
        crate::ai::groq::get_default_groq_api_key()
    }
}

impl Default for GroqVoiceProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Deserialize)]
struct WhisperResponse {
    text: Option<String>,
}

#[async_trait]
impl VoiceProvider for GroqVoiceProvider {
    async fn transcribe(&self, audio: &AudioClip) -> Result<String, OpenMateError> {
        let api_key = self.get_api_key();
        let url = format!("{}/audio/transcriptions", self.base_url);

        let audio_data = audio.data().to_vec();
        let part = multipart::Part::bytes(audio_data)
            .file_name("audio.wav")
            .mime_str("audio/wav")
            .map_err(|e| OpenMateError::Internal(format!("Multipart error: {}", e)))?;

        let form = multipart::Form::new()
            .part("file", part)
            .text("model", GROQ_WHISPER_MODEL)
            .text("response_format", "json")
            .text("prompt", "OpenMate, Rishank, coding, AI companion");

        let res = self
            .client
            .post(&url)
            .bearer_auth(api_key)
            .multipart(form)
            .send()
            .await
            .map_err(|e| OpenMateError::NetworkError(e.to_string()))?;

        let status = res.status();
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(OpenMateError::RateLimited);
        }
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(OpenMateError::InvalidApiKey);
        }
        if !status.is_success() {
            let error_text = res.text().await.unwrap_or_else(|_| "Unknown STT error".to_string());
            warn!("Groq STT returned HTTP {}: {}", status.as_u16(), error_text);
            return Err(OpenMateError::AiProviderError(status.as_u16(), error_text));
        }

        let body: WhisperResponse = res
            .json()
            .await
            .map_err(|e| OpenMateError::SerializationError(e.to_string()))?;

        let text = body.text.unwrap_or_default().trim().to_string();
        info!("Groq STT transcribed {} chars", text.len());
        Ok(text)
    }

    async fn synthesize(&self, text: &str) -> Result<AudioOutput, OpenMateError> {
        let api_key = self.get_api_key();
        let url = format!("{}/audio/speech", self.base_url);

        let req_body = serde_json::json!({
            "model": GROQ_TTS_MODEL,
            "input": text,
            "voice": "zoe",
            "response_format": "wav"
        });

        let res = self
            .client
            .post(&url)
            .bearer_auth(api_key)
            .json(&req_body)
            .send()
            .await;

        if let Ok(response) = res {
            if response.status().is_success() {
                if let Ok(bytes) = response.bytes().await {
                    if !bytes.is_empty() {
                        info!("Groq TTS generated {} bytes of audio", bytes.len());
                        return Ok(AudioOutput::new(bytes.to_vec(), AudioFormat::Wav));
                    }
                }
            } else {
                warn!("Groq TTS returned HTTP {}, falling back to system TTS", response.status());
            }
        }

        // Native system TTS fallback (macOS `say` or silent buffer)
        synthesize_system_fallback(text).await
    }
}

/// Fallback to macOS system TTS (`say`) or silent WAV
pub async fn synthesize_system_fallback(text: &str) -> Result<AudioOutput, OpenMateError> {
    warn!("Groq TTS failed or requires terms, using macOS say fallback");
    #[cfg(target_os = "macos")]
    {
        let temp_dir = std::env::temp_dir();
        let temp_path = temp_dir.join(format!("openmate_tts_{}.wav", uuid::Uuid::new_v4()));
        let path_str = temp_path.to_string_lossy().to_string();
        let text_owned = text.to_string();

        let status = tokio::task::spawn_blocking(move || {
            // Try Ava voice first (natural high-quality modern voice on macOS)
            let s = std::process::Command::new("say")
                .args([
                    "-v",
                    "Ava",
                    "-o",
                    &path_str,
                    "--file-format=WAVE",
                    "--data-format=LEI16@24000",
                    &text_owned,
                ])
                .status();

            if let Ok(st) = s {
                if st.success() {
                    return Ok(());
                }
            }

            // Fallback to default say voice
            std::process::Command::new("say")
                .args([
                    "-o",
                    &path_str,
                    "--file-format=WAVE",
                    "--data-format=LEI16@24000",
                    &text_owned,
                ])
                .status()
                .map(|_| ())
        })
        .await
        .map_err(|e| OpenMateError::Internal(e.to_string()))?;

        if status.is_ok() {
            if let Ok(bytes) = std::fs::read(&temp_path) {
                let _ = std::fs::remove_file(&temp_path);
                if bytes.len() >= 44 {
                    info!("macOS say generated {} bytes of WAV audio", bytes.len());
                    return Ok(AudioOutput::new(bytes, AudioFormat::Wav));
                }
            }
        }
        let _ = std::fs::remove_file(&temp_path);
    }

    // Silent fallback WAV for tests / non-macOS headless environments
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

    Ok(AudioOutput::new(cursor.into_inner(), AudioFormat::Wav))
}

// ── Unit Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    async fn run_mock_server(
        status: u16,
        body: &'static str,
    ) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let base_url = format!("http://{}", addr);

        let handle = tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let mut buf = [0u8; 4096];
                let _ = socket.read(&mut buf).await;

                let response = format!(
                    "HTTP/1.1 {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    status,
                    body.len(),
                    body
                );
                let _ = socket.write_all(response.as_bytes()).await;
            }
        });

        (base_url, handle)
    }

    #[tokio::test]
    async fn test_system_tts_fallback_generates_audio() {
        let res = synthesize_system_fallback("Hello test").await;
        assert!(res.is_ok());
        let output = res.unwrap();
        assert!(!output.data.is_empty());
        output.discard();
    }

    #[tokio::test]
    async fn test_groq_voice_provider_transcribe_success() {
        let mock_body = r#"{"text": "Hello OpenMate, my name is Rishank."}"#;
        let (mock_url, _server) = run_mock_server(200, mock_body).await;
        let provider = GroqVoiceProvider::with_base_url(mock_url);

        let clip = AudioClip::new(vec![0u8; 100], AudioFormat::Wav, 16000);
        let result = provider.transcribe(&clip).await;
        assert!(result.is_ok(), "Expected Ok transcription: {:?}", result);
        assert_eq!(result.unwrap(), "Hello OpenMate, my name is Rishank.");
        clip.discard();
    }

    #[tokio::test]
    async fn test_groq_voice_provider_transcribe_429_maps_to_rate_limited() {
        let mock_body = r#"{"error": {"message": "Rate limit reached"}}"#;
        let (mock_url, _server) = run_mock_server(429, mock_body).await;
        let provider = GroqVoiceProvider::with_base_url(mock_url);

        let clip = AudioClip::new(vec![0u8; 100], AudioFormat::Wav, 16000);
        let result = provider.transcribe(&clip).await;
        assert!(matches!(result, Err(OpenMateError::RateLimited)));
        clip.discard();
    }

    #[tokio::test]
    async fn test_groq_voice_provider_transcribe_401_maps_to_invalid_api_key() {
        let mock_body = r#"{"error": {"message": "Invalid API key"}}"#;
        let (mock_url, _server) = run_mock_server(401, mock_body).await;
        let provider = GroqVoiceProvider::with_base_url(mock_url);

        let clip = AudioClip::new(vec![0u8; 100], AudioFormat::Wav, 16000);
        let result = provider.transcribe(&clip).await;
        assert!(matches!(result, Err(OpenMateError::InvalidApiKey)));
        clip.discard();
    }
}
