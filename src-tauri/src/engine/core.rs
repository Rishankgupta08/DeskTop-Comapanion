//! # OpenMate Core
//!
//! Central orchestrator. Coordinates all engines and routes user input to the
//! appropriate mode handler. [docs/04-SDD.md §3.3]
//!
//! Phase 2-B: Full orchestration loop with tool execution and voice pipeline.

use crate::{
    ai::provider::{AIProvider, ChatMessage, MessageRole},
    engine::{
        context::ContextEngine, memory::MemoryEngine, mode::{CompanionMode, ModeEngine},
        permission::{Capability, PermissionEngine}, tool::{Tool, ToolEngine},
    },
    error::OpenMateError,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info};
use uuid::Uuid;

/// Maximum number of historical messages retrieved for LLM prompt context.
/// DR-019 TBD — update when retention policy confirmed
pub const MAX_HISTORY_TURNS: usize = 20; // DR-019 TBD

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceResult {
    pub transcription: String,
    pub response: String,
}

/// Application-level state held by the core orchestrator.
pub struct AppState {
    pub permission_engine: Arc<PermissionEngine>,
    pub memory_engine: Arc<MemoryEngine>,
    pub mode_engine: Arc<ModeEngine>,
    pub context_engine: Arc<ContextEngine>,
    pub tool_engine: Arc<ToolEngine>,
    pub ai_provider: Arc<dyn AIProvider>,
    pub voice_provider: Arc<dyn crate::ai::voice::VoiceProvider>,
    pub app_handle: Option<tauri::AppHandle>,
    /// Current conversation session ID.
    pub session_id: RwLock<String>,
}

impl AppState {
    pub fn new(
        permission_engine: Arc<PermissionEngine>,
        memory_engine: Arc<MemoryEngine>,
        mode_engine: Arc<ModeEngine>,
        context_engine: Arc<ContextEngine>,
        tool_engine: Arc<ToolEngine>,
        ai_provider: Arc<dyn AIProvider>,
        voice_provider: Arc<dyn crate::ai::voice::VoiceProvider>,
    ) -> Self {
        Self {
            permission_engine,
            memory_engine,
            mode_engine,
            context_engine,
            tool_engine,
            ai_provider,
            voice_provider,
            app_handle: None,
            session_id: RwLock::new(Uuid::new_v4().to_string()),
        }
    }

    pub fn with_handle(mut self, app_handle: tauri::AppHandle) -> Self {
        self.app_handle = Some(app_handle);
        self
    }

    /// Start a new conversation session (clears history context).
    pub async fn new_session(&self) {
        let mut sid = self.session_id.write().await;
        *sid = Uuid::new_v4().to_string();
        info!("Started new conversation session: {}", *sid);
    }

    /// Attempt to parse and execute embedded tool calls in the AI response.
    /// [WRITE_FILE] is intentionally deferred to frontend confirmation UI.
    async fn try_execute_embedded_tools(&self, content: &str) -> Option<String> {
        if let Some(start) = content.find("[READ_FILE:") {
            if let Some(end) = content[start..].find(']') {
                let path_str = content[start + 11..start + end].trim();
                let tool = Tool::ReadFile {
                    path: std::path::PathBuf::from(path_str),
                };
                match self.tool_engine.execute(tool, &self.permission_engine).await {
                    Ok(res) => {
                        return Some(format!(
                            "{}\n\n**File Content ({})**:\n```\n{}\n```",
                            content, path_str, res.output
                        ))
                    }
                    Err(e) => {
                        return Some(format!(
                            "{}\n\n*(Failed to read file {}: {})*",
                            content,
                            path_str,
                            e.user_message()
                        ))
                    }
                }
            }
        }

        if let Some(start) = content.find("[LIST_DIR:") {
            if let Some(end) = content[start..].find(']') {
                let path_str = content[start + 10..start + end].trim();
                let tool = Tool::ListDirectory {
                    path: std::path::PathBuf::from(path_str),
                };
                match self.tool_engine.execute(tool, &self.permission_engine).await {
                    Ok(res) => {
                        return Some(format!(
                            "{}\n\n**Directory Listing ({})**:\n```\n{}\n```",
                            content, path_str, res.output
                        ))
                    }
                    Err(e) => {
                        return Some(format!(
                            "{}\n\n*(Failed to list directory {}: {})*",
                            content,
                            path_str,
                            e.user_message()
                        ))
                    }
                }
            }
        }

        if let Some(start) = content.find("[SEARCH:") {
            if let Some(end) = content[start..].find(']') {
                let raw = &content[start + 8..start + end];
                if let Some((path_str, query_str)) = raw.split_once('|') {
                    let tool = Tool::SearchInFile {
                        path: std::path::PathBuf::from(path_str.trim()),
                        query: query_str.trim().to_string(),
                    };
                    match self.tool_engine.execute(tool, &self.permission_engine).await {
                        Ok(res) => {
                            return Some(format!(
                                "{}\n\n**Search Results ({})**:\n```\n{}\n```",
                                content,
                                path_str.trim(),
                                res.output
                            ))
                        }
                        Err(e) => {
                            return Some(format!(
                                "{}\n\n*(Search failed: {})*",
                                content,
                                e.user_message()
                            ))
                        }
                    }
                }
            }
        }

        if let Some(start) = content.find("[OPEN_APP:") {
            if let Some(end) = content[start..].find(']') {
                let app_name = content[start + 10..start + end].trim();
                let tool = Tool::OpenApplication {
                    name: app_name.to_string(),
                };
                match self.tool_engine.execute(tool, &self.permission_engine).await {
                    Ok(res) => {
                        if res.success {
                            return Some(format!("{}\n\n*(Opened {})*", content, app_name));
                        } else {
                            return Some(format!(
                                "{}\n\n*(Could not open {}: {})*",
                                content,
                                app_name,
                                res.error.unwrap_or_default()
                            ));
                        }
                    }
                    Err(e) => {
                        return Some(format!(
                            "{}\n\n*(App launch error: {})*",
                            content,
                            e.user_message()
                        ))
                    }
                }
            }
        }

        None
    }

    /// Full send_message orchestration pipeline:
    /// 1. Validate input message.
    /// 2. ModeEngine builds system prompt for current/requested mode.
    /// 3. MemoryEngine retrieves relevant history context (capped at MAX_HISTORY_TURNS).
    /// 4. AI Provider executes completion request.
    /// 5. If response suggests tool calls, executes them according to permissions.
    /// 6. MemoryEngine persists the conversation turn.
    /// 7. Returns assistant reply content.
    pub async fn send_message(
        &self,
        message: &str,
        mode_override: Option<CompanionMode>,
    ) -> Result<String, OpenMateError> {
        let trimmed = message.trim();
        if trimmed.is_empty() {
            return Err(OpenMateError::Internal("Message cannot be empty".to_string()));
        }

        // 1. Determine active mode
        self.context_engine.record_user_interaction().await;
        let active_mode = match mode_override {
            Some(m) => {
                self.mode_engine.switch_to(m.clone()).await?;
                m
            }
            None => self.mode_engine.current().await,
        };
        debug!("Processing message in mode: {:?}", active_mode);

        let session_id = self.session_id.read().await.clone();

        // 2. Retrieve history context BEFORE the Gemini call [SRS FR-037]
        let history = self
            .memory_engine
            .get_history(&session_id, MAX_HISTORY_TURNS as u32)
            .await?;

        // 3. Construct message list
        let mut messages: Vec<ChatMessage> = history
            .into_iter()
            .map(|msg| ChatMessage {
                role: if msg.role == "user" {
                    MessageRole::User
                } else {
                    MessageRole::Assistant
                },
                content: msg.content,
            })
            .collect();

        // Add current user turn
        messages.push(ChatMessage {
            role: MessageRole::User,
            content: trimmed.to_string(),
        });

        // 4. Check for and learn personal details/memories [Step 3]
        self.extract_and_persist_personal_memories(trimmed).await;

        let companion_name = self
            .memory_engine
            .get_companion_name()
            .await
            .unwrap_or_else(|_| "OpenMate".to_string());
        let user_name = self.memory_engine.get_user_name().await.unwrap_or_default();

        // 5. Retrieve saved memories for context personalization [Step 3]
        let memories = self.memory_engine.get_memories().await.unwrap_or_default();
        let memory_context = if !memories.is_empty() {
            let formatted_memories: Vec<String> = memories
                .iter()
                .take(10)
                .map(|m| format!("- {}", m.content))
                .collect();
            let target_user = if user_name.trim().is_empty() {
                "the user".to_string()
            } else {
                user_name.trim().to_string()
            };
            Some(format!(
                "Here are things {} has shared with you recently:\n{}\nReference these naturally when relevant. Don't be creepy about it.",
                target_user,
                formatted_memories.join("\n")
            ))
        } else {
            None
        };

        let request = self
            .mode_engine
            .build_request(
                messages,
                memory_context,
                Some(&companion_name),
                Some(&user_name),
            )
            .await;

        // 6. Call AI Provider
        let response = self.ai_provider.generate_text(request).await?;

        // 6. Execute any embedded tools (read, list, search, open_app)
        let final_reply = if let Some(enriched) = self.try_execute_embedded_tools(&response.content).await {
            enriched
        } else {
            response.content
        };

        // 7. Save turn to memory ONLY AFTER the AI call succeeds
        self.memory_engine
            .append_message(&session_id, "user", trimmed)
            .await?;
        self.memory_engine
            .append_message(&session_id, "assistant", &final_reply)
            .await?;

        debug!("Turn persisted to conversation history for session: {}", session_id);

        Ok(final_reply)
    }

    /// Full voice orchestration pipeline: [DR-005, DR-012]
    /// 1. Check PermissionEngine for Capability::Microphone
    /// 2. Acquire PermissionToken
    /// 3. Capture audio clip (5000 ms default)
    /// 4. Transcribe via Gemini STT -> text string
    /// 5. audio_clip.discard()
    /// 6. Call send_message(transcribed_text, None)
    /// 7. Get text response
    /// 8. Generate speech via TTS -> AudioOutput
    /// 9. Play audio in background
    /// 10. Return VoiceResult { transcription, response }
    pub async fn handle_voice_input(&self) -> Result<VoiceResult, OpenMateError> {
        self.context_engine.record_user_interaction().await;
        let token = self
            .permission_engine
            .check(Capability::Microphone)
            .await?;

        let audio_clip = crate::platform::audio::capture_audio_clip(&token, 5000).await?;

        let transcription = match self.voice_provider.transcribe(&audio_clip).await {
            Ok(t) => {
                audio_clip.discard();
                t
            }
            Err(e) => {
                audio_clip.discard();
                return Err(e);
            }
        };

        if transcription.trim().is_empty() {
            return Err(OpenMateError::Internal(
                "No speech detected in audio clip".to_string(),
            ));
        }

        let response = self.send_message(&transcription, None).await?;

        if let Ok(speech_output) = self.voice_provider.synthesize(&response).await {
            let handle_clone = self.app_handle.clone();
            tokio::spawn(async move {
                let _ = crate::platform::audio::play_audio(speech_output, handle_clone).await;
            });
        }

        Ok(VoiceResult {
            transcription,
            response,
        })
    }

    /// Generate a brief ambient notification message from Gemini reflecting cat personality. [Feature 2.3]
    pub async fn generate_ambient_message(&self) -> Result<String, OpenMateError> {
        let companion_name = self
            .memory_engine
            .get_companion_name()
            .await
            .unwrap_or_else(|_| "OpenMate".to_string());
        let user_name = self.memory_engine.get_user_name().await.unwrap_or_default();
        let user = if user_name.trim().is_empty() {
            "your human friend"
        } else {
            user_name.trim()
        };

        let prompt = format!(
            "You are {companion_name}, a friendly, cute cat companion. \
             Generate one short (max 12 words), casual, cat-personality message \
             to send to {user} as an ambient notification. Be playful, warm, \
             or curious. No greetings. Just the message."
        );

        let request = crate::ai::provider::CompletionRequest {
            system_instruction: format!("You are {companion_name}, a cute cat companion."),
            messages: vec![ChatMessage {
                role: MessageRole::User,
                content: prompt,
            }],
            context: None,
        };

        let res = self.ai_provider.generate_text(request).await?;
        let msg = res.content.trim().trim_matches('"').to_string();
        Ok(msg)
    }

    /// Automatically extract user name and personal facts from user input. [Step 3]
    async fn extract_and_persist_personal_memories(&self, user_text: &str) {
        let lower = user_text.to_lowercase();

        // 1. Detect name introductions
        if lower.starts_with("my name is ") {
            let name = user_text[11..].trim().trim_matches('.').trim_matches('!').trim();
            if !name.is_empty() && name.len() < 30 {
                let _ = self.memory_engine.set_user_name(name).await;
                info!("Learned user name: '{}'", name);
            }
        } else if lower.starts_with("call me ") {
            let name = user_text[8..].trim().trim_matches('.').trim_matches('!').trim();
            if !name.is_empty() && name.len() < 30 {
                let _ = self.memory_engine.set_user_name(name).await;
                info!("Learned user name: '{}'", name);
            }
        } else if lower.starts_with("i am ") {
            let rest = lower[5..].trim();
            if !rest.starts_with("busy")
                && !rest.starts_with("tired")
                && !rest.starts_with("happy")
                && !rest.starts_with("sad")
                && !rest.starts_with("working")
                && !rest.starts_with("feeling")
                && !rest.starts_with("trying")
                && !rest.starts_with("going")
            {
                let name = user_text[5..].trim().trim_matches('.').trim_matches('!').trim();
                if !name.is_empty() && name.len() < 25 && !name.contains(' ') {
                    let _ = self.memory_engine.set_user_name(name).await;
                    info!("Learned user name: '{}'", name);
                }
            }
        }

        // 2. Detect personal facts/events
        let is_personal = lower.contains("exam")
            || lower.contains("birthday")
            || lower.contains("favorite")
            || lower.contains("work as")
            || lower.contains("live in")
            || lower.contains("learning")
            || lower.contains("bad day")
            || lower.contains("good day")
            || lower.contains("my dog")
            || lower.contains("my cat")
            || lower.contains("my friend");

        if is_personal && user_text.len() < 200 {
            let entry = crate::engine::memory::NewMemoryEntry {
                content: user_text.trim().to_string(),
                tags: vec!["personal".to_string()],
            };
            let _ = self.memory_engine.save_memory(entry).await;
            info!("Saved personal memory: '{}'", user_text.trim());
        }
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::provider::{CompletionRequest, CompletionResponse, MultimodalRequest};
    use crate::engine::permission::PermissionState;
    use crate::platform::audio::{AudioClip, AudioFormat, AudioOutput};
    use async_trait::async_trait;
    use tokio_rusqlite::Connection;

    struct MockAiProvider {
        succeed: bool,
        reply: String,
        transcription: String,
    }

    #[async_trait]
    impl AIProvider for MockAiProvider {
        async fn generate_text(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, OpenMateError> {
            if self.succeed {
                Ok(CompletionResponse {
                    content: self.reply.clone(),
                    tool_call: None,
                    usage: None,
                })
            } else {
                Err(OpenMateError::InvalidApiKey)
            }
        }

        async fn analyze_image_with_text(
            &self,
            _request: MultimodalRequest,
        ) -> Result<CompletionResponse, OpenMateError> {
            unimplemented!()
        }

        async fn validate_credentials(&self) -> Result<bool, OpenMateError> {
            Ok(self.succeed)
        }

        async fn transcribe_audio(&self, _audio: &AudioClip) -> Result<String, OpenMateError> {
            if self.succeed {
                Ok(self.transcription.clone())
            } else {
                Err(OpenMateError::AiProviderUnavailable)
            }
        }

        async fn generate_speech(&self, _text: &str) -> Result<AudioOutput, OpenMateError> {
            Ok(AudioOutput::new(vec![0; 100], AudioFormat::Wav))
        }
    }

    struct MockVoiceProvider {
        succeed: bool,
        transcription: String,
    }

    #[async_trait::async_trait]
    impl crate::ai::voice::VoiceProvider for MockVoiceProvider {
        async fn transcribe(&self, _audio: &AudioClip) -> Result<String, OpenMateError> {
            if self.succeed {
                Ok(self.transcription.clone())
            } else {
                Err(OpenMateError::AiProviderUnavailable)
            }
        }

        async fn synthesize(&self, _text: &str) -> Result<AudioOutput, OpenMateError> {
            Ok(AudioOutput::new(vec![0; 100], AudioFormat::Wav))
        }
    }

    async fn create_test_state(ai_succeeds: bool, reply: &str) -> AppState {
        let db = Connection::open_in_memory().await.unwrap();
        crate::engine::memory::run_migrations(&db).await.unwrap();

        let db_perm = db.clone();
        let permission_engine = Arc::new(PermissionEngine::new(db_perm).await.unwrap());
        let memory_engine = Arc::new(MemoryEngine::new(db));
        let mode_engine = Arc::new(ModeEngine::new(CompanionMode::Assistant));
        let ai_provider: Arc<dyn AIProvider> = Arc::new(MockAiProvider {
            succeed: ai_succeeds,
            reply: reply.to_string(),
            transcription: "Hello OpenMate from test".to_string(),
        });
        let voice_provider: Arc<dyn crate::ai::voice::VoiceProvider> = Arc::new(MockVoiceProvider {
            succeed: ai_succeeds,
            transcription: "Hello OpenMate from test".to_string(),
        });
        let context_engine = Arc::new(ContextEngine::new(
            Arc::clone(&permission_engine),
            Arc::clone(&ai_provider),
        ));
        let tool_engine = Arc::new(ToolEngine::new(Arc::clone(&permission_engine)));

        AppState::new(
            permission_engine,
            memory_engine,
            mode_engine,
            context_engine,
            tool_engine,
            ai_provider,
            voice_provider,
        )
    }

    #[tokio::test]
    async fn test_failed_ai_call_does_not_save_to_memory() {
        let state = create_test_state(false, "Should not appear").await;
        let session_id = state.session_id.read().await.clone();

        let result = state.send_message("Hello AI", None).await;
        assert!(result.is_err(), "Expected error when AI provider fails");

        let history = state
            .memory_engine
            .get_history(&session_id, 10)
            .await
            .unwrap();

        assert!(
            history.is_empty(),
            "Memory must not contain any turns if AI call failed"
        );
    }

    #[tokio::test]
    async fn test_successful_ai_call_persists_both_turns() {
        let state = create_test_state(true, "Hello human!").await;
        let session_id = state.session_id.read().await.clone();

        let result = state.send_message("Hello AI", None).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "Hello human!");

        let history = state
            .memory_engine
            .get_history(&session_id, 10)
            .await
            .unwrap();

        assert_eq!(history.len(), 2, "Expected user + assistant turns in history");
        assert_eq!(history[0].role, "user");
        assert_eq!(history[0].content, "Hello AI");
        assert_eq!(history[1].role, "assistant");
        assert_eq!(history[1].content, "Hello human!");
    }

    #[tokio::test]
    async fn test_voice_input_requires_microphone_permission() {
        let state = create_test_state(true, "Voice reply").await;
        // Default permission is Off
        let res = state.handle_voice_input().await;
        assert!(
            matches!(res, Err(OpenMateError::PermissionDenied(_))),
            "Expected PermissionDenied when Microphone is Off, got: {:?}",
            res
        );
    }

    #[tokio::test]
    async fn test_voice_input_flow_when_permission_allowed() {
        let state = create_test_state(true, "Voice reply").await;
        state
            .permission_engine
            .set_permission(Capability::Microphone, PermissionState::Allow)
            .await
            .unwrap();

        let res = state.handle_voice_input().await;
        assert!(res.is_ok(), "Expected voice input to succeed: {:?}", res);
        let val = res.unwrap();
        assert_eq!(val.transcription, "Hello OpenMate from test");
        assert_eq!(val.response, "Voice reply");
    }

    #[tokio::test]
    async fn test_voice_input_fails_when_transcription_fails_no_fake_fallback() {
        let state = create_test_state(false, "Voice reply").await;
        state
            .permission_engine
            .set_permission(Capability::Microphone, PermissionState::Allow)
            .await
            .unwrap();

        let res = state.handle_voice_input().await;
        assert!(res.is_err());
        assert!(matches!(res, Err(OpenMateError::AiProviderUnavailable)));
    }

    #[tokio::test]
    async fn test_voice_input_fails_when_transcription_is_empty() {
        let db = Connection::open_in_memory().await.unwrap();
        crate::engine::memory::run_migrations(&db).await.unwrap();

        let db_perm = db.clone();
        let permission_engine = Arc::new(PermissionEngine::new(db_perm).await.unwrap());
        permission_engine
            .set_permission(Capability::Microphone, PermissionState::Allow)
            .await
            .unwrap();

        let memory_engine = Arc::new(MemoryEngine::new(db));
        let mode_engine = Arc::new(ModeEngine::new(CompanionMode::Assistant));
        let ai_provider: Arc<dyn AIProvider> = Arc::new(MockAiProvider {
            succeed: true,
            reply: "Voice reply".to_string(),
            transcription: "   ".to_string(), // Empty whitespace
        });
        let voice_provider: Arc<dyn crate::ai::voice::VoiceProvider> = Arc::new(MockVoiceProvider {
            succeed: true,
            transcription: "   ".to_string(), // Empty whitespace
        });
        let context_engine = Arc::new(ContextEngine::new(
            Arc::clone(&permission_engine),
            Arc::clone(&ai_provider),
        ));
        let tool_engine = Arc::new(ToolEngine::new(Arc::clone(&permission_engine)));

        let state = AppState::new(
            permission_engine,
            memory_engine,
            mode_engine,
            context_engine,
            tool_engine,
            ai_provider,
            voice_provider,
        );

        let res = state.handle_voice_input().await;
        assert!(res.is_err());
        if let Err(e) = res {
            assert!(
                e.user_message().contains("No speech"),
                "Expected clear speech detection message, got: {}",
                e.user_message()
            );
        }
    }

    #[tokio::test]
    async fn test_automatic_user_name_learning_and_memory_extraction() {
        let db = Connection::open_in_memory().await.unwrap();
        crate::engine::memory::run_migrations(&db).await.unwrap();

        let db_perm = db.clone();
        let permission_engine = Arc::new(PermissionEngine::new(db_perm).await.unwrap());
        let memory_engine = Arc::new(MemoryEngine::new(db));
        let mode_engine = Arc::new(ModeEngine::new(CompanionMode::PersonalFriend));
        let ai_provider: Arc<dyn AIProvider> = Arc::new(MockAiProvider {
            succeed: true,
            reply: "Nice to meet you Rishank! *purrs*".to_string(),
            transcription: "".to_string(),
        });
        let voice_provider: Arc<dyn crate::ai::voice::VoiceProvider> = Arc::new(MockVoiceProvider {
            succeed: true,
            transcription: "".to_string(),
        });
        let context_engine = Arc::new(ContextEngine::new(
            Arc::clone(&permission_engine),
            Arc::clone(&ai_provider),
        ));
        let tool_engine = Arc::new(ToolEngine::new(Arc::clone(&permission_engine)));

        let state = AppState::new(
            permission_engine,
            Arc::clone(&memory_engine),
            mode_engine,
            context_engine,
            tool_engine,
            ai_provider,
            voice_provider,
        );

        // 1. Send "My name is Rishank"
        let _ = state.send_message("My name is Rishank", None).await.unwrap();
        let user_name = memory_engine.get_user_name().await.unwrap();
        assert_eq!(user_name, "Rishank");

        // 2. Send "I have an exam tomorrow"
        let _ = state.send_message("I have an exam tomorrow", None).await.unwrap();
        let memories = memory_engine.get_memories().await.unwrap();
        assert!(!memories.is_empty());
        assert!(memories.iter().any(|m| m.content.contains("exam tomorrow")));
    }
}
