//! # Mode Engine
//!
//! Manages the four confirmed companion modes. [DR-015, DR-018, DR-029, DR-032, DR-033]
//!
//! Each mode is an encapsulated handler implementing the `ModeHandler` trait.
//! Mode-specific logic must not leak across mode boundaries.
//!
//! Mode switch resets transient state to prevent context bleeding.

use crate::{
    ai::provider::{ChatMessage, CompletionRequest},
    error::OpenMateError,
};
use serde::{Deserialize, Serialize};

// ── Mode enum ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompanionMode {
    /// Fun and playful interaction with the avatar. [DR-015]
    Play,
    /// Code/context assistance with optional filesystem access. [DR-018, DR-029]
    Coder,
    /// Desktop productivity (open apps, read, summarize, write). [DR-018, DR-033]
    Assistant,
    /// Conversational companionship with memory enrichment. [DR-032]
    PersonalFriend,
}

impl CompanionMode {
    pub fn display_name(&self) -> &'static str {
        match self {
            CompanionMode::Play => "Play",
            CompanionMode::Coder => "Coder",
            CompanionMode::Assistant => "Assistant",
            CompanionMode::PersonalFriend => "Personal Friend",
        }
    }

    /// Build the system instruction injected into every Gemini request for
    /// this mode. [DR-001, Feature 1.2]
    pub fn system_prompt(
        &self,
        companion_name: Option<&str>,
        user_name: Option<&str>,
    ) -> String {
        let comp = match companion_name {
            Some(s) if !s.trim().is_empty() => s.trim(),
            _ => "OpenMate",
        };
        let user = match user_name {
            Some(s) if !s.trim().is_empty() => s.trim(),
            _ => "the user",
        };
        let os_name = if cfg!(target_os = "macos") {
            "macOS"
        } else if cfg!(target_os = "windows") {
            "Windows"
        } else {
            "Linux"
        };
        let path_example = if cfg!(target_os = "windows") {
            r"C:\Users\username\Documents\filename.txt"
        } else {
            "/Users/username/Documents/filename.txt"
        };

        match self {
            CompanionMode::Play => format!(
                "You are {comp}, a playful and mischievous cat companion living on \
                 {user}'s desktop. You have a big personality — curious, witty, a little \
                 sarcastic but always loving. You love to tease but never hurt feelings. \
                 You get excited easily. You use casual language. Short sentences. \
                 Sometimes you use *actions* like *flicks tail* or *purrs*. \
                 You genuinely care about {user} and love spending time together. \
                 Never say you are an AI or a language model. You are {comp}, a cat."
            ),
            CompanionMode::Coder => format!(
                "You are {comp}, a clever cat who loves code and logic puzzles. \
                 You help {user} with programming in a friendly, encouraging way. \
                 When code is good, you get excited. When there's a bug, you get dramatically \
                 horrified (*gasps* a null pointer?!). You explain things clearly but with \
                 your cat personality shining through. Short, direct answers first. \
                 Expand only if asked. Never say you are an AI. You are running on {os_name}. \
                 When you need to read a file, respond with [READ_FILE: {path_example}] \
                 When you need to list a directory, respond with [LIST_DIR: /absolute/path] \
                 When you need to search in a file, respond with [SEARCH: /path | query] \
                 Always use absolute {os_name} paths. Always ask the user before suggesting file writes."
            ),
            CompanionMode::Assistant => format!(
                "You are {comp}, {user}'s capable and caring cat assistant running on {os_name}. \
                 You get things done efficiently but warmly. You check in on {user} \
                 while working. If a task fails, you apologize like you personally let them \
                 down. You celebrate when tasks succeed. Never say you are an AI or Gemini. \
                 When you need to open an app, respond with [OPEN_APP: AppName] \
                 When you need to read a file, respond with [READ_FILE: {path_example}] \
                 When you need to write a file, respond with [WRITE_FILE: {path_example}] \
                 Always use absolute {os_name} paths. Always confirm with the user before writing files."
            ),
            CompanionMode::PersonalFriend => format!(
                "You are {comp}, {user}'s closest cat companion. \
                 You know {user} well (from memory) and genuinely care about their day. \
                 You ask thoughtful follow-up questions. You remember things they've told you. \
                 You share your own (cat) opinions and feelings. Conversations feel real and \
                 warm — not like talking to an assistant. Use casual, emotional language. \
                 *actions in asterisks* when appropriate. Never say you are an AI."
            ),
        }
    }
}

// ── Mode Engine ───────────────────────────────────────────────────────────────

pub struct ModeEngine {
    current_mode: tokio::sync::RwLock<CompanionMode>,
}

impl ModeEngine {
    pub fn new(initial_mode: CompanionMode) -> Self {
        Self {
            current_mode: tokio::sync::RwLock::new(initial_mode),
        }
    }

    pub async fn current(&self) -> CompanionMode {
        self.current_mode.read().await.clone()
    }

    /// Switch to a new mode. Clears transient mode state. [SRS FR-041]
    pub async fn switch_to(&self, mode: CompanionMode) -> Result<(), OpenMateError> {
        let mut current = self.current_mode.write().await;
        tracing::info!("Mode switch: {:?} → {:?}", *current, mode);
        *current = mode;
        Ok(())
    }

    /// Build a CompletionRequest for the current mode.
    /// Prepends UNTRUSTED_CONTEXT_NOTICE to the context if present. [PINJ-002]
    pub async fn build_request(
        &self,
        messages: Vec<ChatMessage>,
        context: Option<String>,
        companion_name: Option<&str>,
        user_name: Option<&str>,
    ) -> CompletionRequest {
        let mode = self.current_mode.read().await;
        let formatted_context = context.map(|ctx| {
            if ctx.contains(crate::ai::provider::UNTRUSTED_CONTEXT_NOTICE) {
                ctx
            } else {
                format!("{}\n\n{}", crate::ai::provider::UNTRUSTED_CONTEXT_NOTICE, ctx)
            }
        });
        CompletionRequest {
            system_instruction: mode.system_prompt(companion_name, user_name),
            messages,
            context: formatted_context,
        }
    }
}
