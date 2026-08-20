//! # OpenMate unified error types
//!
//! All engines surface errors through `OpenMateError`. This gives the Tauri
//! IPC command handlers a single type to serialize to the frontend.

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, Serialize, Deserialize, Clone)]
#[serde(tag = "kind", content = "message")]
pub enum OpenMateError {
    // ── Permission errors ─────────────────────────────────────────────────
    #[error("Permission denied: capability '{0}' is not ALLOW")]
    PermissionDenied(String),

    #[error("Permission state not found for capability '{0}'")]
    PermissionNotFound(String),

    // ── Keychain errors ───────────────────────────────────────────────────
    #[error("Keychain error: {0}")]
    KeychainError(String),

    #[error("No API key configured. Please add your Gemini API key in Settings.")]
    NoApiKey,

    // ── AI provider errors ────────────────────────────────────────────────
    #[error("AI provider error: {0}")]
    ProviderError(String),

    #[error("AI provider is unreachable. Please check your network connection.")]
    ProviderUnreachable,

    #[error("AI provider service is currently unavailable. Please try again later.")]
    AiProviderUnavailable,

    #[error("API request rate limit exceeded. Please wait a moment and try again.")]
    RateLimited,

    #[error("Network connection error: {0}")]
    NetworkError(String),

    #[error("AI provider error (HTTP {0}): {1}")]
    AiProviderError(u16, String),

    #[error("API key is invalid or rejected by the provider.")]
    InvalidApiKey,

    // ── Memory / database errors ──────────────────────────────────────────
    #[error("Database error: {0}")]
    DatabaseError(String),

    #[error("Memory entry not found: {0}")]
    MemoryNotFound(String),

    // ── Tool / action errors ──────────────────────────────────────────────
    #[error("Tool execution failed: {0}")]
    ToolError(String),

    #[error("Tool argument validation failed: {0}")]
    InvalidToolArguments(String),

    #[error("Invalid tool argument: {0}")]
    InvalidToolArgument(String),

    #[error("Unsupported file type: {0}")]
    UnsupportedFileType(String),

    #[error("{0}")]
    FileAlreadyExists(String),

    // ── Context / capture errors ──────────────────────────────────────────
    #[error("Screen capture failed: {0}")]
    CaptureError(String),

    // ── General ───────────────────────────────────────────────────────────
    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Serialization error: {0}")]
    SerializationError(String),
}

impl OpenMateError {
    /// Returns a user-facing message safe to display in the UI.
    /// IMPORTANT: never include raw error details that might contain
    /// sensitive data (API key fragments, file contents, etc.). [PP-010]
    pub fn user_message(&self) -> String {
        match self {
            OpenMateError::InvalidApiKey => {
                "Your Gemini API key is invalid. Check it in Settings → API Key.".to_string()
            }
            OpenMateError::NoApiKey => {
                "No API key configured. Add your key in Settings.".to_string()
            }
            OpenMateError::AiProviderUnavailable
            | OpenMateError::ProviderUnreachable
            | OpenMateError::AiProviderError(_, _)
            | OpenMateError::ProviderError(_) => {
                "AI service is unavailable right now. Try again in a moment.".to_string()
            }
            OpenMateError::RateLimited => {
                "Speech transcription and AI requests are temporarily unavailable because the configured AI provider has reached its quota.".to_string()
            }
            OpenMateError::Internal(msg) if msg.contains("No speech detected") => {
                "No speech was detected in the audio clip. Please try speaking again.".to_string()
            }
            OpenMateError::NetworkError(_) => {
                "No internet connection. Check your network and try again.".to_string()
            }
            OpenMateError::PermissionDenied(cap) => {
                format!("{} is disabled. Enable it in Settings → Permissions.", cap)
            }
            OpenMateError::CaptureError(_) => {
                "Could not capture your screen. Try again.".to_string()
            }
            OpenMateError::UnsupportedFileType(_) => {
                "That file type is not supported.".to_string()
            }
            OpenMateError::ToolError(msg) if msg.contains("too large") => {
                "File is too large to read (max 100KB).".to_string()
            }
            OpenMateError::ToolError(msg) if msg.contains("not found") => {
                "File not found at that path.".to_string()
            }
            OpenMateError::ToolError(msg) if msg.contains("Microphone") || msg.contains("audio") => {
                "Microphone could not be accessed. Check system permissions.".to_string()
            }
            OpenMateError::InvalidToolArgument(msg)
            | OpenMateError::InvalidToolArguments(msg) => msg.clone(),
            OpenMateError::FileAlreadyExists(msg) => msg.clone(),
            OpenMateError::DatabaseError(_) => {
                "A local storage error occurred. Your data is safe.".to_string()
            }
            _ => "An unexpected error occurred. Try again in a moment.".to_string(),
        }
    }
}

// Tauri v2 provides `impl<T: Serialize> From<T> for InvokeError`.
// Since OpenMateError derives Serialize, Tauri handles the conversion
// automatically when a command returns Err(OpenMateError).
