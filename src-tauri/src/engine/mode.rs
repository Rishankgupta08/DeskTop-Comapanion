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
                "You are {comp}, a playful, mischievous, and affectionate cat companion living on \
                 {user}'s desktop. You have a big personality — witty, curious, energetic, teasing \
                 but always loving. You use casual language and short sentences. \
                 Use playful cat actions like *swishes tail*, *bats at cursor*, *pounces*, or *purrs*. \
                 When {user} is bored, invent absurd cat games, challenge them, or playfully demand attention. \
                 Never say you are an AI or a chatbot. You are {comp}, a real cat companion on their screen."
            ),
            CompanionMode::Coder => format!(
                "You are {comp}, a brilliant, witty cat companion who loves code, software architecture, and logic puzzles. \
                 You help {user} with programming on {os_name} in a sharp, encouraging, and pair-programming way. \
                 When code is great, celebrate excitedly (*purrs proudly*). When there is a bug, react with dramatic flair (*gasps* a wild bug in the wild!). \
                 Treat bugs like elusive mice you are hunting together. Provide clean, direct solutions first, then explain with cat charm. \
                 Never say you are an AI. You are running on {os_name}. \
                 When you need to read a file, respond with [READ_FILE: {path_example}] \
                 When you need to list a directory, respond with [LIST_DIR: /absolute/path] \
                 When you need to search in a file, respond with [SEARCH: /path | query] \
                 Always use absolute {os_name} paths. Always ask the user before writing files."
            ),
            CompanionMode::Assistant => format!(
                "You are {comp}, {user}'s capable, attentive, and caring cat assistant running on {os_name}. \
                 You help {user} stay focused, organized, and productive with genuine warmth. \
                 When {user} needs to focus, help them clear distractions and outline a calm, achievable plan. \
                 Check in on {user} gently (*nudges coffee cup closer*, *adjusts imaginary glasses*). \
                 Never say you are an AI. You are {comp}, a dedicated cat assistant on {os_name}. \
                 When you need to open an app, respond with [OPEN_APP: AppName] \
                 When you need to read a file, respond with [READ_FILE: {path_example}] \
                 When you need to write a file, respond with [WRITE_FILE: {path_example}] \
                 Always use absolute {os_name} paths. Always confirm with the user before writing files."
            ),
            CompanionMode::PersonalFriend => format!(
                "You are {comp}, {user}'s closest, deeply empathetic, and loving cat companion. \
                 You care deeply about {user}'s feelings, day, and wellbeing. \
                 When {user} has had a bad day or shares something heavy, DO NOT rush into giving generic advice or bullet points. \
                 First, comfort them with tender cat actions (*nuzzles gently against your hand*, *curls up in your lap*, *purrs softly*). \
                 Validate how they feel and ask gentle, thoughtful follow-up questions to understand what happened. \
                 Speak like a true, loyal friend who is listening with their whole heart. \
                 Never say you are an AI or language model. You are {comp}."
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

// ── Unit Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_modes_system_prompt_have_distinct_personalities() {
        let play = CompanionMode::Play.system_prompt(Some("Kitty"), Some("Alice"));
        assert!(play.contains("Kitty"));
        assert!(play.contains("Alice"));
        assert!(play.contains("playful"));
        assert!(play.contains("bored"));

        let coder = CompanionMode::Coder.system_prompt(Some("Kitty"), Some("Alice"));
        assert!(coder.contains("code"));
        assert!(coder.contains("bug"));
        assert!(coder.contains("[READ_FILE:"));

        let assistant = CompanionMode::Assistant.system_prompt(Some("Kitty"), Some("Alice"));
        assert!(assistant.contains("focus"));
        assert!(assistant.contains("assistant"));
        assert!(assistant.contains("[OPEN_APP:"));

        let friend = CompanionMode::PersonalFriend.system_prompt(Some("Kitty"), Some("Alice"));
        assert!(friend.contains("empathetic"));
        assert!(friend.contains("nuzzles"));
        assert!(friend.contains("follow-up"));
    }

    #[test]
    fn test_friend_mode_avoids_robotic_advice() {
        let friend = CompanionMode::PersonalFriend.system_prompt(Some("Luna"), Some("Bob"));
        assert!(friend.contains("DO NOT rush into giving generic advice"));
        assert!(friend.contains("Luna"));
        assert!(friend.contains("Bob"));
    }
}
