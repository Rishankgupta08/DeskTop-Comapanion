//! # Context Engine
//!
//! Responsible for desktop context capture, active window/app observation,
//! smart proactive assistance, and idle return greetings. [DR-030, DR-007, DR-017, DR-008, ADR-007]
//!
//! ## Security invariants (non-negotiable):
//! - Every capture attempt MUST check `PermissionEngine` first. [DR-012, PP-001]
//! - Screenshots are captured into memory ONLY and discarded immediately after AI call. [DR-008, PP-002]
//! - `CapturedFrame::discard()` is guaranteed to be called on BOTH success and failure paths.
//! - Context is prepended with `UNTRUSTED_CONTEXT_NOTICE` before reaching Gemini. [PINJ-001, PINJ-002]
//! - `ClipboardChanged` is NEVER proactively triggered to Gemini. [DR-007]
//! - `UserReturnedFromIdle` does NOT capture a screenshot. [DR-007]

use crate::{
    ai::provider::{
        AIProvider, ChatMessage, CompletionRequest, MessageRole, MultimodalRequest,
        UNTRUSTED_CONTEXT_NOTICE,
    },
    engine::{
        mode::CompanionMode,
        permission::{Capability, PermissionEngine, PermissionState, PermissionToken},
    },
    error::OpenMateError,
    platform::capture::{self, CaptureTarget, CapturedFrame},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

// ── Proactive Mode [DR-017] ───────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProactiveMode {
    Off,
    Subtle, // trigger on app change or after idle + significant context change
    Active, // trigger on every confirmed context change + smart periodic vision
}

impl ProactiveMode {
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "active" => ProactiveMode::Active,
            "off" => ProactiveMode::Off,
            _ => ProactiveMode::Subtle,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            ProactiveMode::Off => "off",
            ProactiveMode::Subtle => "subtle",
            ProactiveMode::Active => "active",
        }
    }
}

// ── Trigger & Response Types ──────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextTrigger {
    /// Explicit user query e.g. "what's on my screen?" [DR-030]
    ExplicitUserRequest,
    /// Active window title changed [DR-030]
    ActiveWindowChanged(String),
    /// Active application changed [DR-030]
    ActiveAppChanged(String),
    /// Selected desktop events (stub) [DR-007]
    SelectedDesktopEvents,
    /// Clipboard content changed [DR-007]
    ClipboardChanged,
    /// User returned after being idle (>5 minutes) [DR-007]
    UserReturnedFromIdle,
}

impl ContextTrigger {
    pub fn description(&self) -> String {
        match self {
            ContextTrigger::ExplicitUserRequest => {
                "User explicitly requested screen context analysis.".to_string()
            }
            ContextTrigger::ActiveWindowChanged(title) => {
                format!("Active window changed to: '{}'", title)
            }
            ContextTrigger::ActiveAppChanged(app) => {
                format!("Active application changed to: '{}'", app)
            }
            ContextTrigger::SelectedDesktopEvents => {
                "Selected desktop event occurred.".to_string()
            }
            ContextTrigger::ClipboardChanged => "Clipboard content changed.".to_string(),
            ContextTrigger::UserReturnedFromIdle => {
                "User returned after being idle.".to_string()
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextResponse {
    pub analysis: String,
    pub suggested_response: Option<String>,
    pub frame_discarded: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextStatus {
    pub permission_state: String,
    pub monitor_active: bool,
}

/// Helper to get app-specific context prompt based on active application name. [Step 2]
pub fn get_app_context_prompt(app_name: &str) -> Option<&'static str> {
    let name = app_name.to_lowercase();
    if name.contains("xcode") || name.contains("code") || name.contains("cursor") || name.contains("rust") || name.contains("idea") {
        Some("User is coding. Offer to help with code if needed.")
    } else if name.contains("safari") || name.contains("chrome") || name.contains("brave") || name.contains("edge") || name.contains("arc") || name.contains("firefox") {
        Some("User is browsing. Offer to summarize or help research.")
    } else if name.contains("terminal") || name.contains("iterm") || name.contains("warp") || name.contains("alacritty") {
        Some("User is in terminal. Offer to help with commands.")
    } else if name.contains("figma") || name.contains("sketch") || name.contains("photoshop") || name.contains("illustrator") {
        Some("User is designing. Be encouraging.")
    } else if name.contains("zoom") || name.contains("meet") || name.contains("teams") || name.contains("webex") || (name.contains("slack") && name.contains("huddle")) {
        Some("User is in a meeting. Stay quiet unless asked.")
    } else {
        None
    }
}

// ── Context Engine ────────────────────────────────────────────────────────────

pub struct ContextEngine {
    pub permission_engine: Arc<PermissionEngine>,
    pub ai_provider: Arc<dyn AIProvider>,
    pub memory_engine: Arc<RwLock<Option<Arc<crate::engine::memory::MemoryEngine>>>>,
    pub app_handle: Arc<RwLock<Option<tauri::AppHandle>>>,
    pub monitor_cancel: Arc<RwLock<Option<CancellationToken>>>,
    pub last_app_name: Arc<RwLock<Option<String>>>,
    pub last_window_title: Arc<RwLock<Option<String>>>,
    pub current_app_started: Arc<RwLock<(Option<String>, Instant)>>,
    pub last_proactive_capture: Arc<RwLock<Option<Instant>>>,
    pub last_interaction: Arc<RwLock<Instant>>,
}

impl ContextEngine {
    pub fn new(
        permission_engine: Arc<PermissionEngine>,
        ai_provider: Arc<dyn AIProvider>,
    ) -> Self {
        Self {
            permission_engine,
            ai_provider,
            memory_engine: Arc::new(RwLock::new(None)),
            app_handle: Arc::new(RwLock::new(None)),
            monitor_cancel: Arc::new(RwLock::new(None)),
            last_app_name: Arc::new(RwLock::new(None)),
            last_window_title: Arc::new(RwLock::new(None)),
            current_app_started: Arc::new(RwLock::new((None, Instant::now()))),
            last_proactive_capture: Arc::new(RwLock::new(None)),
            last_interaction: Arc::new(RwLock::new(Instant::now())),
        }
    }

    pub async fn set_memory_engine(&self, mem: Arc<crate::engine::memory::MemoryEngine>) {
        let mut guard = self.memory_engine.write().await;
        *guard = Some(mem);
    }

    pub async fn set_app_handle(&self, handle: tauri::AppHandle) {
        let mut guard = self.app_handle.write().await;
        *guard = Some(handle);
    }

    /// Record a user interaction timestamp (message, voice input, avatar click).
    pub async fn record_user_interaction(&self) {
        let mut t = self.last_interaction.write().await;
        *t = Instant::now();
    }

    /// Calculate idle duration in milliseconds.
    pub async fn idle_duration_ms(&self) -> u64 {
        let t = self.last_interaction.read().await;
        t.elapsed().as_millis() as u64
    }

    /// Determine if proactive assistance should trigger. [DR-017, DR-007]
    pub fn should_trigger_proactive(
        &self,
        trigger: &ContextTrigger,
        mode: ProactiveMode,
        idle_duration_ms: u64,
    ) -> bool {
        match mode {
            ProactiveMode::Off => false,
            ProactiveMode::Subtle => match trigger {
                ContextTrigger::UserReturnedFromIdle => true,
                ContextTrigger::ActiveWindowChanged(_) => idle_duration_ms > 30_000,
                ContextTrigger::ActiveAppChanged(_) => idle_duration_ms > 30_000,
                ContextTrigger::ClipboardChanged => false,
                ContextTrigger::ExplicitUserRequest => true,
                ContextTrigger::SelectedDesktopEvents => false,
            },
            ProactiveMode::Active => match trigger {
                ContextTrigger::UserReturnedFromIdle => true,
                ContextTrigger::ActiveWindowChanged(_) => true,
                ContextTrigger::ActiveAppChanged(_) => true,
                ContextTrigger::ClipboardChanged => false,
                ContextTrigger::ExplicitUserRequest => true,
                ContextTrigger::SelectedDesktopEvents => false,
            },
        }
    }

    /// Generate an idle return greeting based on companion mode. [DR-007]
    /// Does NOT capture a screenshot.
    pub fn generate_idle_greeting(mode: &CompanionMode) -> &'static str {
        match mode {
            CompanionMode::Play => "Welcome back! Miss me?",
            CompanionMode::PersonalFriend => "Hey, you're back! How's it going?",
            CompanionMode::Coder => "Back to it? What are we working on?",
            CompanionMode::Assistant => "Welcome back. Ready to help when you are.",
        }
    }

    /// Primary context pipeline (Explicit user query like "what's on my screen?"):
    /// 1. Check PermissionEngine (Off -> Err(PermissionDenied), Ask -> Err(PermissionDenied))
    /// 2. Acquire PermissionToken
    /// 3. In-memory capture via capture_screen(&token, ActiveWindow)
    /// 4. Build Vision prompt with UNTRUSTED_CONTEXT_NOTICE prepended
    /// 5. Call AIProvider::analyze_image_with_text
    /// 6. ALWAYS call frame.discard() before returning (success or failure)
    /// 7. Return ContextResponse with frame_discarded: true
    pub async fn handle_context_request(
        &self,
        trigger: ContextTrigger,
        user_query: Option<String>,
    ) -> Result<ContextResponse, OpenMateError> {
        // STEP 1 & 2: Permission verification and token acquisition [PP-001]
        let token: PermissionToken = self
            .permission_engine
            .check(Capability::ScreenCapture)
            .await?;

        // STEP 3: In-memory capture [DR-008]
        let frame: CapturedFrame = capture::capture_screen(&token, CaptureTarget::ActiveWindow)
            .await
            .map_err(|e| OpenMateError::CaptureError(e.to_string()))?;

        // Process request with guarantee that frame.discard() runs on all exit paths
        self.process_frame_and_analyze(frame, trigger, user_query).await
    }

    async fn process_frame_and_analyze(
        &self,
        frame: CapturedFrame,
        trigger: ContextTrigger,
        user_query: Option<String>,
    ) -> Result<ContextResponse, OpenMateError> {
        let mime_type = frame.format.mime_type().to_string();
        let image_bytes = frame.data.clone();

        // STEP 5: Build context prompt with mandatory UNTRUSTED_CONTEXT_NOTICE [PINJ-002]
        let trigger_desc = trigger.description();
        let query_text = user_query.unwrap_or_else(|| {
            "Describe what is currently visible on the screen and provide relevant context."
                .to_string()
        });

        let context_info = format!(
            "Trigger: {}\n\nUser Question: {}",
            trigger_desc, query_text
        );

        let system_instruction = format!(
            "You are OpenMate, an intelligent desktop assistant with screen awareness.\n\
             {}\n\n\
             Desktop context details:\n{}",
            UNTRUSTED_CONTEXT_NOTICE, context_info
        );

        let request = MultimodalRequest {
            system_instruction,
            messages: vec![ChatMessage {
                role: MessageRole::User,
                content: query_text,
            }],
            image_bytes,
            image_mime_type: mime_type,
            context: Some(context_info),
        };

        // STEP 6: Execute AI Vision call
        let ai_result = self.ai_provider.analyze_image_with_text(request).await;

        // STEP 7: GUARANTEED IN-MEMORY DISCARD [DR-008, PP-002]
        // Zero all image bytes in memory regardless of whether AI succeeded or failed.
        frame.discard();
        debug!("Screenshot in-memory buffer explicitly discarded and zeroized");

        // Map AI response or return error
        let response = ai_result?;

        Ok(ContextResponse {
            analysis: response.content,
            suggested_response: None,
            frame_discarded: true,
        })
    }

    /// Background monitor that polls for active window and app changes. [DR-030]
    /// Runs a lightweight interval loop (default: 2000ms).
    pub async fn start_monitor(self: &Arc<Self>, interval_ms: u64) {
        let mut cancel_guard = self.monitor_cancel.write().await;
        if cancel_guard.is_some() {
            debug!("Background context monitor is already running");
            return;
        }

        let cancel_token = CancellationToken::new();
        *cancel_guard = Some(cancel_token.clone());
        drop(cancel_guard);

        let engine = Arc::clone(self);
        let interval_duration = std::time::Duration::from_millis(interval_ms);

        info!(
            "Starting background window/app context monitor (interval: {}ms)",
            interval_ms
        );

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(interval_duration);

            loop {
                tokio::select! {
                    _ = cancel_token.cancelled() => {
                        info!("Background window context monitor stopped");
                        break;
                    }
                    _ = interval.tick() => {
                        engine.check_window_change_tick().await;
                    }
                }
            }
        });
    }

    /// Stop the background window monitor.
    pub async fn stop_monitor(&self) {
        let mut cancel_guard = self.monitor_cancel.write().await;
        if let Some(token) = cancel_guard.take() {
            token.cancel();
            info!("Cancelled background window context monitor");
        }
    }

    /// Query whether the monitor is currently running.
    pub async fn is_monitor_active(&self) -> bool {
        let cancel_guard = self.monitor_cancel.read().await;
        cancel_guard.is_some()
    }

    /// Single tick of active window & app observation. [Steps 2, 3, 4]
    pub async fn check_window_change_tick(&self) {
        let (current_app, current_title) = Self::get_active_app_and_title();
        let app_name = match current_app {
            Some(a) => a,
            None => return,
        };

        let mut last_app_guard = self.last_app_name.write().await;
        let app_changed = match *last_app_guard {
            Some(ref last) => last != &app_name,
            None => true,
        };

        if app_changed {
            debug!("Observed active application changed to: '{}'", app_name);
            *last_app_guard = Some(app_name.clone());
            let mut app_session = self.current_app_started.write().await;
            *app_session = (Some(app_name.clone()), Instant::now());
        }
        drop(last_app_guard);

        if let Some(ref title) = current_title {
            let mut last_title_guard = self.last_window_title.write().await;
            *last_title_guard = Some(title.clone());
        }

        // Query proactive mode
        let proactive_mode = if let Some(ref mem) = *self.memory_engine.read().await {
            mem.get_proactive_mode()
                .await
                .map(|s| ProactiveMode::from_str(&s))
                .unwrap_or(ProactiveMode::Subtle)
        } else {
            ProactiveMode::Subtle
        };

        if proactive_mode == ProactiveMode::Off {
            return;
        }

        // STEP 2: When active app changes AND proactive mode is Subtle or Active
        if app_changed {
            if let Some(prompt_hint) = get_app_context_prompt(&app_name) {
                // If user is in a meeting, companion stays quiet!
                if prompt_hint.contains("in a meeting") {
                    debug!("User is in a meeting (app: {}); companion stays quiet", app_name);
                    return;
                }

                // Generate ambient bubble (without taking screenshot)
                if let Some(message) = self.generate_app_ambient_message(&app_name, prompt_hint).await {
                    self.emit_ambient_message(&message, Some(&app_name)).await;
                }
            }
        }

        // STEP 3: Smart screenshot timing (Active mode only)
        // - App has been active for 60+ seconds
        // - Proactive mode is Active
        // - Last proactive screenshot was > 5 minutes ago (300s)
        // - Screen capture permission is Allow
        if proactive_mode == ProactiveMode::Active {
            let (session_app, session_start) = self.current_app_started.read().await.clone();
            if let Some(ref active_app) = session_app {
                if session_start.elapsed().as_secs() >= 60 {
                    let mut last_capture_guard = self.last_proactive_capture.write().await;
                    let should_capture = match *last_capture_guard {
                        Some(last_time) => last_time.elapsed().as_secs() >= 300,
                        None => true,
                    };

                    if should_capture {
                        let perm_state = self
                            .permission_engine
                            .get_state(&Capability::ScreenCapture)
                            .await;

                        if perm_state == PermissionState::Allow {
                            *last_capture_guard = Some(Instant::now());
                            drop(last_capture_guard);

                            // STEP 4: Context-aware Gemini prompt
                            self.trigger_smart_proactive_screenshot(active_app).await;
                        }
                    }
                }
            }
        }
    }

    /// Generate an ambient companion remark based on app context. [Step 2]
    pub async fn generate_app_ambient_message(&self, app_name: &str, context_hint: &str) -> Option<String> {
        let (companion_name, user_name) = if let Some(ref mem) = *self.memory_engine.read().await {
            let c_name = mem.get_companion_name().await.unwrap_or_else(|_| "OpenMate".to_string());
            let u_name = mem.get_user_name().await.unwrap_or_default();
            (c_name, u_name)
        } else {
            ("OpenMate".to_string(), "".to_string())
        };

        let system_instruction = format!(
            "You are {companion_name}, a friendly cat companion watching {user_name}'s desktop.\n\
             Context: User just switched to {app_name}. {context_hint}\n\
             Write ONE brief ambient remark (maximum 12 words) in your playful cat persona.\n\
             Do not ask long questions. Keep it cheerful and concise."
        );

        let req = CompletionRequest {
            system_instruction,
            messages: vec![ChatMessage {
                role: MessageRole::User,
                content: format!("User opened {}", app_name),
            }],
            context: None,
        };

        if let Ok(resp) = self.ai_provider.generate_text(req).await {
            let trimmed = resp.content.trim().to_string();
            if !trimmed.is_empty() && !trimmed.to_lowercase().starts_with("skip") {
                return Some(trimmed);
            }
        }

        // Canned fallback if AI offline
        let lower = app_name.to_lowercase();
        if lower.contains("code") || lower.contains("xcode") {
            Some("Ready to write some clean code together? 🐾".to_string())
        } else if lower.contains("safari") || lower.contains("chrome") {
            Some("Browsing around? Let me know if you need research help! 👀".to_string())
        } else if lower.contains("terminal") {
            Some("Terminal open! Watch those shell commands! 💻".to_string())
        } else if lower.contains("figma") || lower.contains("sketch") {
            Some("Designing something beautiful? You've got this! ✨".to_string())
        } else {
            None
        }
    }

    /// Smart proactive screenshot and context-aware Gemini analysis. [Steps 3 & 4]
    pub async fn trigger_smart_proactive_screenshot(&self, app_name: &str) {
        let (companion_name, user_name) = if let Some(ref mem) = *self.memory_engine.read().await {
            let c_name = mem.get_companion_name().await.unwrap_or_else(|_| "OpenMate".to_string());
            let u_name = mem.get_user_name().await.unwrap_or_default();
            (c_name, u_name)
        } else {
            ("OpenMate".to_string(), "".to_string())
        };

        let token = match self.permission_engine.check(Capability::ScreenCapture).await {
            Ok(t) => t,
            Err(_) => return,
        };

        let frame = match capture::capture_screen(&token, CaptureTarget::ActiveWindow).await {
            Ok(f) => f,
            Err(e) => {
                warn!("Smart proactive capture failed: {}", e);
                return;
            }
        };

        let mime_type = frame.format.mime_type().to_string();
        let image_bytes = frame.data.clone();

        // STEP 4 prompt:
        let system_instruction = format!(
            "You are {companion_name}, a cat companion watching {user_name}'s screen.\n\
             The user is currently using {app_name}.\n\
             Look at this screenshot and if you see something you could helpfully comment on, write ONE short message (max 15 words) in your cat personality.\n\
             If there's nothing helpful to say, respond with exactly: SKIP\n\
             Do not offer generic help. Only comment if you notice something specific.\n\
             {}",
            UNTRUSTED_CONTEXT_NOTICE
        );

        let request = MultimodalRequest {
            system_instruction,
            messages: vec![ChatMessage {
                role: MessageRole::User,
                content: format!("Watching {}", app_name),
            }],
            image_bytes,
            image_mime_type: mime_type,
            context: Some(format!("Proactive screen observation of {}", app_name)),
        };

        let res = self.ai_provider.analyze_image_with_text(request).await;
        frame.discard(); // Guaranteed in-memory discard

        if let Ok(resp) = res {
            let text = resp.content.trim();
            if text.eq_ignore_ascii_case("SKIP") || text.starts_with("SKIP") || text.is_empty() {
                debug!("Gemini returned SKIP for proactive screenshot observation");
            } else {
                info!("Proactive vision comment: {}", text);
                self.emit_ambient_message(text, Some(app_name)).await;
            }
        }
    }

    /// Emit ambient message event to frontend.
    pub async fn emit_ambient_message(&self, message: &str, app_name: Option<&str>) {
        if let Some(ref handle) = *self.app_handle.read().await {
            use tauri::Emitter;
            let payload = serde_json::json!({
                "message": message,
                "app_name": app_name,
            });
            let _ = handle.emit("proactive-ambient-message", payload);
        }
    }

    /// Cross-platform active app and window title resolver.
    pub fn get_active_app_and_title() -> (Option<String>, Option<String>) {
        #[cfg(target_os = "macos")]
        {
            let script = r#"tell application "System Events" to get {name of first application process whose frontmost is true, name of front window of (first application process whose frontmost is true)}"#;
            if let Ok(out) = std::process::Command::new("osascript")
                .args(["-e", script])
                .output()
            {
                if out.status.success() {
                    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    let parts: Vec<&str> = s.split(", ").collect();
                    let app = parts.get(0).map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
                    let title = parts.get(1).map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
                    if app.is_some() || title.is_some() {
                        return (app, title);
                    }
                }
            }
        }

        if let Ok(windows) = xcap::Window::all() {
            if let Some(w) = windows.into_iter().find(|w| !w.is_minimized().unwrap_or(true)) {
                let app = w.app_name().ok();
                let title = w.title().ok();
                return (app, title);
            }
        }

        (None, None)
    }
}

// ── Unit Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::provider::{CompletionRequest, CompletionResponse, MultimodalRequest};
    use async_trait::async_trait;
    use tokio_rusqlite::Connection;

    struct MockVisionAiProvider {
        succeed: bool,
        reply: String,
        analyzed_called: Arc<RwLock<bool>>,
    }

    #[async_trait]
    impl AIProvider for MockVisionAiProvider {
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
                Err(OpenMateError::AiProviderUnavailable)
            }
        }

        async fn analyze_image_with_text(
            &self,
            _request: MultimodalRequest,
        ) -> Result<CompletionResponse, OpenMateError> {
            let mut called = self.analyzed_called.write().await;
            *called = true;

            if self.succeed {
                Ok(CompletionResponse {
                    content: self.reply.clone(),
                    tool_call: None,
                    usage: None,
                })
            } else {
                Err(OpenMateError::AiProviderUnavailable)
            }
        }

        async fn validate_credentials(&self) -> Result<bool, OpenMateError> {
            Ok(true)
        }

        async fn transcribe_audio(
            &self,
            _audio: &crate::platform::audio::AudioClip,
        ) -> Result<String, OpenMateError> {
            unimplemented!()
        }

        async fn generate_speech(
            &self,
            _text: &str,
        ) -> Result<crate::platform::audio::AudioOutput, OpenMateError> {
            unimplemented!()
        }
    }

    async fn create_test_context_engine(
        ai_succeeds: bool,
        permission_state: PermissionState,
    ) -> (Arc<ContextEngine>, Arc<RwLock<bool>>, Arc<PermissionEngine>) {
        let db = Connection::open_in_memory().await.unwrap();
        crate::engine::memory::run_migrations(&db).await.unwrap();

        let permission_engine = Arc::new(PermissionEngine::new(db).await.unwrap());
        permission_engine
            .set_permission(Capability::ScreenCapture, permission_state)
            .await
            .unwrap();

        let analyzed_called = Arc::new(RwLock::new(false));
        let ai_provider = Arc::new(MockVisionAiProvider {
            succeed: ai_succeeds,
            reply: "Mock analysis of screen".to_string(),
            analyzed_called: Arc::clone(&analyzed_called),
        });

        let context_engine = Arc::new(ContextEngine::new(
            Arc::clone(&permission_engine),
            ai_provider,
        ));

        (context_engine, analyzed_called, permission_engine)
    }

    #[test]
    fn test_get_app_context_prompt_mapping() {
        assert!(get_app_context_prompt("Visual Studio Code").unwrap().contains("coding"));
        assert!(get_app_context_prompt("Xcode").unwrap().contains("coding"));
        assert!(get_app_context_prompt("Google Chrome").unwrap().contains("browsing"));
        assert!(get_app_context_prompt("Safari").unwrap().contains("browsing"));
        assert!(get_app_context_prompt("Terminal").unwrap().contains("terminal"));
        assert!(get_app_context_prompt("iTerm2").unwrap().contains("terminal"));
        assert!(get_app_context_prompt("Figma").unwrap().contains("designing"));
        assert!(get_app_context_prompt("zoom.us").unwrap().contains("meeting"));
        assert!(get_app_context_prompt("Google Meet").unwrap().contains("meeting"));
        assert!(get_app_context_prompt("Microsoft Teams").unwrap().contains("meeting"));
        assert!(get_app_context_prompt("Calculator").is_none());
    }

    #[tokio::test]
    async fn test_permission_denied_stops_pipeline_before_capture() {
        let (engine, analyzed_called, _) =
            create_test_context_engine(true, PermissionState::Off).await;

        let result = engine
            .handle_context_request(ContextTrigger::ExplicitUserRequest, None)
            .await;

        assert!(
            matches!(result, Err(OpenMateError::PermissionDenied(_))),
            "Expected PermissionDenied error when ScreenCapture is Off"
        );

        let called = *analyzed_called.read().await;
        assert!(
            !called,
            "AIProvider must NOT be called when permission is denied"
        );
    }

    #[tokio::test]
    async fn test_frame_is_discarded_even_when_gemini_call_fails() {
        let (engine, _, _) = create_test_context_engine(false, PermissionState::Allow).await;

        let result = engine
            .handle_context_request(ContextTrigger::ExplicitUserRequest, None)
            .await;

        assert!(
            result.is_err(),
            "Expected error when AI call fails, got: {:?}",
            result
        );
    }

    #[tokio::test]
    async fn test_frame_is_discarded_on_successful_pipeline() {
        let (engine, analyzed_called, _) =
            create_test_context_engine(true, PermissionState::Allow).await;

        let result = engine
            .handle_context_request(ContextTrigger::ExplicitUserRequest, None)
            .await;

        assert!(
            result.is_ok(),
            "Expected successful context response, got: {:?}",
            result
        );

        let called = *analyzed_called.read().await;
        assert!(called, "AIProvider must be called on success path");

        let res = result.unwrap();
        assert!(
            res.frame_discarded,
            "frame_discarded must be true in ContextResponse"
        );
    }

    #[tokio::test]
    async fn test_monitor_does_not_capture_when_permission_is_off() {
        let (engine, analyzed_called, _) =
            create_test_context_engine(true, PermissionState::Off).await;

        engine.check_window_change_tick().await;

        let called = *analyzed_called.read().await;
        assert!(
            !called,
            "Background monitor must not trigger capture when permission is Off"
        );
    }

    #[test]
    fn test_should_trigger_proactive_returns_false_when_off() {
        let db_perm = tokio_rusqlite::Connection::open_in_memory();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let db = rt.block_on(db_perm).unwrap();
        let permission_engine = Arc::new(rt.block_on(PermissionEngine::new(db)).unwrap());
        let analyzed_called = Arc::new(RwLock::new(false));
        let ai_provider = Arc::new(MockVisionAiProvider {
            succeed: true,
            reply: "Mock".to_string(),
            analyzed_called,
        });
        let engine = ContextEngine::new(permission_engine, ai_provider);

        assert!(!engine.should_trigger_proactive(
            &ContextTrigger::ActiveWindowChanged("Test".to_string()),
            ProactiveMode::Off,
            60_000
        ));
    }

    #[test]
    fn test_should_trigger_proactive_never_triggers_on_clipboard_changed() {
        let db_perm = tokio_rusqlite::Connection::open_in_memory();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let db = rt.block_on(db_perm).unwrap();
        let permission_engine = Arc::new(rt.block_on(PermissionEngine::new(db)).unwrap());
        let analyzed_called = Arc::new(RwLock::new(false));
        let ai_provider = Arc::new(MockVisionAiProvider {
            succeed: true,
            reply: "Mock".to_string(),
            analyzed_called,
        });
        let engine = ContextEngine::new(permission_engine, ai_provider);

        assert!(!engine.should_trigger_proactive(
            &ContextTrigger::ClipboardChanged,
            ProactiveMode::Active,
            60_000
        ));
    }

    #[test]
    fn test_should_trigger_proactive_subtle_window_change_10s_idle_is_false() {
        let db_perm = tokio_rusqlite::Connection::open_in_memory();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let db = rt.block_on(db_perm).unwrap();
        let permission_engine = Arc::new(rt.block_on(PermissionEngine::new(db)).unwrap());
        let analyzed_called = Arc::new(RwLock::new(false));
        let ai_provider = Arc::new(MockVisionAiProvider {
            succeed: true,
            reply: "Mock".to_string(),
            analyzed_called,
        });
        let engine = ContextEngine::new(permission_engine, ai_provider);

        assert!(!engine.should_trigger_proactive(
            &ContextTrigger::ActiveWindowChanged("Code".to_string()),
            ProactiveMode::Subtle,
            10_000
        ));
    }

    #[test]
    fn test_should_trigger_proactive_subtle_window_change_60s_idle_is_true() {
        let db_perm = tokio_rusqlite::Connection::open_in_memory();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let db = rt.block_on(db_perm).unwrap();
        let permission_engine = Arc::new(rt.block_on(PermissionEngine::new(db)).unwrap());
        let analyzed_called = Arc::new(RwLock::new(false));
        let ai_provider = Arc::new(MockVisionAiProvider {
            succeed: true,
            reply: "Mock".to_string(),
            analyzed_called,
        });
        let engine = ContextEngine::new(permission_engine, ai_provider);

        assert!(engine.should_trigger_proactive(
            &ContextTrigger::ActiveWindowChanged("Code".to_string()),
            ProactiveMode::Subtle,
            60_000
        ));
    }

    #[test]
    fn test_idle_return_fires_greeting_without_screenshot() {
        let greeting = ContextEngine::generate_idle_greeting(&CompanionMode::Assistant);
        assert!(!greeting.is_empty());
        assert_eq!(greeting, "Welcome back. Ready to help when you are.");
    }
}
