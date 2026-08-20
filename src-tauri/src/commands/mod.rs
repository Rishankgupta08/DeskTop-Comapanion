//! # Tauri IPC Command Handlers
//!
//! All commands are thin wrappers around engine calls. They:
//! 1. Accept only serializable parameters.
//! 2. Pass work to the appropriate engine.
//! 3. Return serializable DTOs or `OpenMateError` (serialized to JSON).
//!
//! ## Security rule:
//! - No command sends the API key to the frontend.
//! - No command bypasses the PermissionEngine.
//! - Sensitive capabilities return only success/error — never raw data that
//!   reveals system state beyond what the user explicitly requested.
//!
//! ## PermissionToken pattern note:
//! Commands that trigger sensitive OS calls call `permission_engine.check()`
//! which returns a `PermissionToken`. That token is passed down to the native
//! function. The IPC command itself never touches the token — it is internal
//! to the Rust call stack and is never serialized or sent across IPC. [ADR-010]

use crate::{
    engine::{
        memory::{NewMemoryEntry, MemoryEntry, ConversationMessage},
        mode::CompanionMode,
        permission::{Capability, PermissionState},
    },
    error::OpenMateError,
    platform::keychain,
};
use tauri::State;

use crate::AppStateContainer;

// ── Permission commands ───────────────────────────────────────────────────────

/// Get the current permission state for all capabilities.
/// [SRS FR-030, docs/05-security-privacy.md §3.3]
#[tauri::command]
pub async fn get_permissions(
    state: State<'_, AppStateContainer>,
) -> Result<Vec<(Capability, PermissionState)>, OpenMateError> {
    let app = state.0.read().await;
    let states = app.permission_engine.get_all_states().await;
    Ok(states.into_iter().collect())
}

/// Update a capability's permission state.
/// [SRS FR-031, docs/05-security-privacy.md §3.2]
///
/// Note: the frontend may only set Off / Ask / Allow — it cannot grant itself
/// a PermissionToken directly. The token is always issued by the engine. [ADR-010]
#[tauri::command]
pub async fn set_permission(
    state: State<'_, AppStateContainer>,
    capability: Capability,
    new_state: PermissionState,
) -> Result<(), OpenMateError> {
    let app = state.0.read().await;
    app.permission_engine
        .set_permission(capability, new_state)
        .await
}

// ── API key commands ──────────────────────────────────────────────────────────

/// Check whether an API key is stored.
/// Returns boolean ONLY — never the key value. [PP-004, docs/05-security-privacy.md §4.3]
#[tauri::command]
pub async fn has_api_key() -> Result<bool, OpenMateError> {
    Ok(keychain::has_api_key())
}

/// Store the user's Gemini API key in the OS keychain.
/// The key string is consumed immediately. Never logged. [PP-004, PP-010]
#[tauri::command]
pub async fn set_api_key(key: String) -> Result<(), OpenMateError> {
    keychain::set_api_key(&key)
}

/// Delete the stored Gemini API key from the OS keychain.
/// [SRS FR-027, docs/05-security-privacy.md §4.4]
#[tauri::command]
pub async fn delete_api_key() -> Result<(), OpenMateError> {
    keychain::delete_api_key()
}

/// Validate the stored API key by calling the provider's validation endpoint.
/// Returns true if valid and reachable, false otherwise.
/// [SRS FR-028]
#[tauri::command]
pub async fn validate_api_key(
    state: State<'_, AppStateContainer>,
) -> Result<bool, OpenMateError> {
    let app = state.0.read().await;
    app.ai_provider.validate_credentials().await
}

// ── Mode commands ─────────────────────────────────────────────────────────────

/// Get the current companion mode.
#[tauri::command]
pub async fn get_mode(
    state: State<'_, AppStateContainer>,
) -> Result<CompanionMode, OpenMateError> {
    let app = state.0.read().await;
    Ok(app.mode_engine.current().await)
}

/// Switch the companion to a different mode.
/// [SRS FR-041]
#[tauri::command]
pub async fn set_mode(
    state: State<'_, AppStateContainer>,
    mode: CompanionMode,
) -> Result<(), OpenMateError> {
    let app = state.0.read().await;
    app.mode_engine.switch_to(mode).await
}

// ── Memory commands ───────────────────────────────────────────────────────────

/// Retrieve all saved user memories.
/// [SRS FR-037]
#[tauri::command]
pub async fn get_memories(
    state: State<'_, AppStateContainer>,
) -> Result<Vec<MemoryEntry>, OpenMateError> {
    let app = state.0.read().await;
    app.memory_engine.get_memories().await
}

/// Save a new user memory entry.
/// [SRS FR-036]
#[tauri::command]
pub async fn save_memory(
    state: State<'_, AppStateContainer>,
    content: String,
    tags: Vec<String>,
) -> Result<MemoryEntry, OpenMateError> {
    let app = state.0.read().await;
    app.memory_engine
        .save_memory(NewMemoryEntry { content, tags })
        .await
}

/// Delete a specific user memory entry by ID.
/// [SRS FR-038]
#[tauri::command]
pub async fn delete_memory(
    state: State<'_, AppStateContainer>,
    id: String,
) -> Result<(), OpenMateError> {
    let app = state.0.read().await;
    app.memory_engine.delete_memory(id).await
}

/// Clear all user memory entries.
/// [SRS FR-039] — requires explicit user confirmation on the frontend before calling.
#[tauri::command]
pub async fn clear_memories(
    state: State<'_, AppStateContainer>,
) -> Result<u64, OpenMateError> {
    let app = state.0.read().await;
    app.memory_engine.clear_memories().await
}

// ── Conversation commands ─────────────────────────────────────────────────────

/// Get recent conversation history for the current session.
#[tauri::command]
pub async fn get_conversation_history(
    state: State<'_, AppStateContainer>,
    limit: Option<u32>,
) -> Result<Vec<ConversationMessage>, OpenMateError> {
    let app = state.0.read().await;
    let session_id = app.session_id.read().await.clone();
    app.memory_engine
        .get_history(&session_id, limit.unwrap_or(50))
        .await
}

/// Start a new conversation session (clears in-memory history context).
#[tauri::command]
pub async fn new_session(
    state: State<'_, AppStateContainer>,
) -> Result<String, OpenMateError> {
    let app = state.0.read().await;
    app.new_session().await;
    let session_id = app.session_id.read().await.clone();
    Ok(session_id)
}

// ── Chat command ──────────────────────────────────────────────────────────────

/// Send a user message and receive a reply from the AI.
/// Calls OpenMate Core orchestration loop and converts errors to user-readable strings.
/// [SRS FR-020]
#[tauri::command]
pub async fn send_message(
    state: State<'_, AppStateContainer>,
    message: String,
    mode: Option<String>,
) -> Result<String, String> {
    let app = state.0.read().await;

    let parsed_mode = if let Some(m_str) = mode {
        match m_str.to_lowercase().as_str() {
            "play" => Some(CompanionMode::Play),
            "coder" => Some(CompanionMode::Coder),
            "assistant" => Some(CompanionMode::Assistant),
            "personal_friend" | "personalfriend" => Some(CompanionMode::PersonalFriend),
            _ => None,
        }
    } else {
        None
    };

    app.send_message(&message, parsed_mode)
        .await
        .map_err(|e| e.user_message().to_string())
}

// ── Context / Screen commands ─────────────────────────────────────────────────

/// Trigger explicit screen context analysis with the user's query. [DR-030]
/// Returns Gemini Vision analysis string.
#[tauri::command]
pub async fn request_screen_context(
    state: State<'_, AppStateContainer>,
    query: Option<String>,
) -> Result<String, String> {
    let app = state.0.read().await;
    let resp = app
        .context_engine
        .handle_context_request(
            crate::engine::context::ContextTrigger::ExplicitUserRequest,
            query,
        )
        .await
        .map_err(|e| e.user_message().to_string())?;

    Ok(resp.analysis)
}

/// Retrieve screen awareness permission and background monitor state.
#[tauri::command]
pub async fn get_context_status(
    state: State<'_, AppStateContainer>,
) -> Result<crate::engine::context::ContextStatus, String> {
    let app = state.0.read().await;
    let perm_state = app
        .permission_engine
        .get_state(&Capability::ScreenCapture)
        .await;
    let monitor_active = app.context_engine.is_monitor_active().await;

    Ok(crate::engine::context::ContextStatus {
        permission_state: perm_state.as_str().to_string(),
        monitor_active,
    })
}

// ── Tool commands ─────────────────────────────────────────────────────────────

/// Execute a policy-gated native tool. [DR-018, DR-033]
#[tauri::command]
pub async fn execute_tool(
    state: State<'_, AppStateContainer>,
    tool_name: String,
    args: std::collections::HashMap<String, String>,
) -> Result<crate::engine::tool::ToolResult, String> {
    let app = state.0.read().await;

    let tool = match tool_name.as_str() {
        "open_application" => {
            let name = args.get("name").cloned().unwrap_or_default();
            crate::engine::tool::Tool::OpenApplication { name }
        }
        "read_file" => {
            let path_str = args.get("path").cloned().unwrap_or_default();
            crate::engine::tool::Tool::ReadFile {
                path: std::path::PathBuf::from(path_str),
            }
        }
        "write_file" => {
            let path_str = args.get("path").cloned().unwrap_or_default();
            let content = args.get("content").cloned().unwrap_or_default();
            let overwrite = args
                .get("overwrite")
                .map(|v| v == "true" || v == "1")
                .unwrap_or(false);
            let create_copy = args
                .get("createCopy")
                .or_else(|| args.get("create_copy"))
                .map(|v| v == "true" || v == "1")
                .unwrap_or(false);
            crate::engine::tool::Tool::WriteFile {
                path: std::path::PathBuf::from(path_str),
                content,
                overwrite,
                create_copy,
            }
        }
        "list_directory" => {
            let path_str = args.get("path").cloned().unwrap_or_default();
            crate::engine::tool::Tool::ListDirectory {
                path: std::path::PathBuf::from(path_str),
            }
        }
        "search_in_file" => {
            let path_str = args.get("path").cloned().unwrap_or_default();
            let query = args.get("query").cloned().unwrap_or_default();
            crate::engine::tool::Tool::SearchInFile {
                path: std::path::PathBuf::from(path_str),
                query,
            }
        }
        _ => return Err(format!("Unknown tool '{}'", tool_name)),
    };

    app.tool_engine
        .execute(tool, &app.permission_engine)
        .await
        .map_err(|e| e.user_message().to_string())
}

// ── Voice commands (Phase 2-B) ────────────────────────────────────────────────

/// Start voice input pipeline: records audio, transcribes with Gemini, runs conversation turn, and plays TTS.
/// [DR-005, DR-012]
#[tauri::command]
pub async fn start_voice_input(
    state: State<'_, AppStateContainer>,
) -> Result<crate::engine::core::VoiceResult, String> {
    let app = state.0.read().await;
    app.handle_voice_input()
        .await
        .map_err(|e| e.user_message().to_string())
}

// ── Proactive commands (Phase 2-C) ────────────────────────────────────────────

/// Get current proactive assistance mode ('off', 'subtle', 'active'). [DR-017]
#[tauri::command]
pub async fn get_proactive_mode(
    state: State<'_, AppStateContainer>,
) -> Result<String, String> {
    let app = state.0.read().await;
    app.memory_engine
        .get_proactive_mode()
        .await
        .map_err(|e| e.user_message().to_string())
}

/// Set proactive assistance mode. [DR-017]
#[tauri::command]
pub async fn set_proactive_mode(
    state: State<'_, AppStateContainer>,
    mode: String,
) -> Result<(), String> {
    let app = state.0.read().await;
    app.memory_engine
        .set_proactive_mode(&mode)
        .await
        .map_err(|e| e.user_message().to_string())
}

// ── Identity & Ambient commands (Feature 1 & Feature 2) ──────────────────────

/// Retrieve the configured companion name.
#[tauri::command]
pub async fn get_companion_name(
    state: State<'_, AppStateContainer>,
) -> Result<String, String> {
    let app = state.0.read().await;
    app.memory_engine
        .get_companion_name()
        .await
        .map_err(|e| e.user_message().to_string())
}

/// Set the companion name.
#[tauri::command]
pub async fn set_companion_name(
    state: State<'_, AppStateContainer>,
    name: String,
) -> Result<(), String> {
    let app = state.0.read().await;
    app.memory_engine
        .set_companion_name(&name)
        .await
        .map_err(|e| e.user_message().to_string())
}

/// Retrieve the configured user name.
#[tauri::command]
pub async fn get_user_name(
    state: State<'_, AppStateContainer>,
) -> Result<String, String> {
    let app = state.0.read().await;
    app.memory_engine
        .get_user_name()
        .await
        .map_err(|e| e.user_message().to_string())
}

/// Set the user name.
#[tauri::command]
pub async fn set_user_name(
    state: State<'_, AppStateContainer>,
    name: String,
) -> Result<(), String> {
    let app = state.0.read().await;
    app.memory_engine
        .set_user_name(&name)
        .await
        .map_err(|e| e.user_message().to_string())
}

/// Generate a dynamic ambient bubble message via Gemini.
#[tauri::command]
pub async fn generate_ambient_message(
    state: State<'_, AppStateContainer>,
) -> Result<String, String> {
    let app = state.0.read().await;
    app.generate_ambient_message()
        .await
        .map_err(|e| e.user_message().to_string())
}

/// Dynamically resize and reposition the native window based on UI layout state or custom dimensions.
#[tauri::command]
pub async fn set_window_layout_state(
    window: tauri::Window,
    layout_state: String,
    custom_width: Option<f64>,
    custom_height: Option<f64>,
) -> Result<(), String> {
    let (target_w, target_h) = match (custom_width, custom_height) {
        (Some(w), Some(h)) if w > 0.0 && h > 0.0 => (w, h),
        _ => match layout_state.as_str() {
            "CHAT" => (420.0, 640.0),
            "SETTINGS" => (680.0, 750.0),
            "BUBBLE" => (320.0, 200.0),
            _ => (120.0, 120.0),
        },
    };

    let scale_factor = window
        .scale_factor()
        .map_err(|e| format!("Failed to get scale_factor: {}", e))?;

    let phys_pos = window
        .outer_position()
        .map_err(|e| format!("Failed to get outer_position: {}", e))?;

    let phys_size = window
        .inner_size()
        .map_err(|e| format!("Failed to get inner_size: {}", e))?;

    let old_w = phys_size.width as f64 / scale_factor;
    let old_h = phys_size.height as f64 / scale_factor;

    let cur_x = phys_pos.x as f64 / scale_factor;
    let cur_y = phys_pos.y as f64 / scale_factor;

    // Retrieve current monitor work area for boundary clamping
    let (min_x, min_y, max_x, max_y) = if let Ok(Some(monitor)) = window.current_monitor() {
        let mon_pos = monitor.position();
        let mon_size = monitor.size();
        let m_min_x = mon_pos.x as f64 / scale_factor;
        let m_min_y = mon_pos.y as f64 / scale_factor;
        let m_max_x = m_min_x + (mon_size.width as f64 / scale_factor);
        let m_max_y = m_min_y + (mon_size.height as f64 / scale_factor);
        (m_min_x, m_min_y, m_max_x, m_max_y)
    } else {
        (0.0, 0.0, 1920.0, 1080.0)
    };

    let anchor_x = cur_x + old_w;
    let anchor_y = cur_y + old_h;

    let mut new_x = anchor_x - target_w;
    let mut new_y = anchor_y - target_h;

    // Clamp to monitor boundaries so companion is never pushed off-screen
    if new_x < min_x {
        new_x = min_x.max(cur_x);
    }
    if new_x + target_w > max_x {
        new_x = (max_x - target_w).max(min_x);
    }
    if new_y < min_y {
        new_y = min_y.max(cur_y);
    }
    if new_y + target_h > max_y {
        new_y = (max_y - target_h).max(min_y);
    }

    tracing::info!(
        "Native window layout update: state={}, current=({:.1}x{:.1}) at ({:.1},{:.1}), target=({:.1}x{:.1}) at ({:.1},{:.1}), monitor bounds=({:.1},{:.1}) to ({:.1},{:.1})",
        layout_state, old_w, old_h, cur_x, cur_y, target_w, target_h, new_x, new_y, min_x, min_y, max_x, max_y
    );

    if target_w > old_w || target_h > old_h {
        window
            .set_position(tauri::LogicalPosition::new(new_x, new_y))
            .map_err(|e| format!("Failed to set position: {}", e))?;
        window
            .set_size(tauri::LogicalSize::new(target_w, target_h))
            .map_err(|e| format!("Failed to set size: {}", e))?;
    } else {
        window
            .set_size(tauri::LogicalSize::new(target_w, target_h))
            .map_err(|e| format!("Failed to set size: {}", e))?;
        window
            .set_position(tauri::LogicalPosition::new(new_x, new_y))
            .map_err(|e| format!("Failed to set position: {}", e))?;
    }

    Ok(())
}

/// Trigger native window drag from the backend.
#[tauri::command]
pub async fn start_window_drag(window: tauri::Window) -> Result<(), String> {
    window
        .start_dragging()
        .map_err(|e| format!("Failed to start dragging: {}", e))
}
