//! # OpenMate — Tauri library crate root
//!
//! Initializes all engines, wires state into the Tauri runtime, and registers
//! all IPC command handlers. [docs/04-SDD.md §3.3]

pub mod ai;
pub mod commands;
pub mod engine;
pub mod error;
pub mod platform;

use crate::{
    ai::gemini::GeminiProvider,
    engine::{
        context::ContextEngine,
        core::AppState,
        memory::MemoryEngine,
        mode::{CompanionMode, ModeEngine},
        permission::PermissionEngine,
        tool::ToolEngine,
    },
};
use std::sync::Arc;
use tauri::Manager;
use tokio::sync::RwLock;
use tracing_subscriber::{EnvFilter, FmtSubscriber};

/// Newtype wrapping `AppState` for Tauri's managed state.
/// The `RwLock` allows multiple concurrent reads from command handlers.
pub struct AppStateContainer(pub RwLock<AppState>);

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // ── Load .env environment variables if present ────────────────────────
    dotenvy::dotenv().ok();

    // ── Logging (local only — no telemetry) [DR-028] ──────────────────────
    let subscriber = FmtSubscriber::builder()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .finish();
    tracing::subscriber::set_global_default(subscriber)
        .expect("Failed to set global tracing subscriber");

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_shortcuts(["CmdOrCtrl+Shift+Space"])
                .unwrap()
                .with_handler(|app, _shortcut, event| {
                    if event.state() == tauri_plugin_global_shortcut::ShortcutState::Pressed {
                        use tauri::Emitter;
                        tracing::info!("Global shortcut Cmd+Shift+Space triggered voice input");
                        let _ = app.emit("trigger-voice-input", ());
                        let _ = app.emit("global-voice-trigger", ());
                    }
                })
                .build(),
        )
        .setup(|app| {
            let handle = app.handle().clone();

            // Resolve app_data_dir at runtime from Tauri path resolver [DR-006]
            let app_data = app
                .path()
                .app_data_dir()
                .map_err(|e| format!("Failed to resolve app_data_dir: {}", e))?;
            std::fs::create_dir_all(&app_data)
                .map_err(|e| format!("Failed to create app_data_dir: {}", e))?;
            let db_path = app_data.join("openmate.db");
            tracing::info!("OpenMate database path: {:?}", db_path); // [Fix-3.2]

            // Spawn async initialization on the Tauri async runtime.
            let handle_for_state = handle.clone();
            tauri::async_runtime::spawn(async move {
                match initialize_engines(db_path, Some(handle_for_state.clone())).await {
                    Ok(state) => {
                        handle_for_state.manage(AppStateContainer(RwLock::new(state)));
                        tracing::info!("OpenMate engines initialized successfully");
                    }
                    Err(e) => {
                        tracing::error!("Fatal: engine initialization failed: {:?}", e);
                        // In V1 we panic — Phase 1-F will add graceful startup error UX.
                        panic!("Engine initialization failed: {:?}", e);
                    }
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // Permission commands
            commands::get_permissions,
            commands::set_permission,
            // API key commands [PP-004]
            commands::has_api_key,
            commands::set_api_key,
            commands::delete_api_key,
            commands::validate_api_key,
            // Mode commands
            commands::get_mode,
            commands::set_mode,
            // Memory commands
            commands::get_memories,
            commands::save_memory,
            commands::delete_memory,
            commands::clear_memories,
            // Conversation commands
            commands::get_conversation_history,
            commands::new_session,
            // Chat commands (Phase 1-C)
            commands::send_message,
            // Context / Screen commands (Phase 1-E)
            commands::request_screen_context,
            commands::get_context_status,
            // Tool commands (Phase 2-A)
            commands::execute_tool,
            // Voice commands (Phase 2-B)
            commands::start_voice_input,
            // Proactive commands (Phase 2-C)
            commands::get_proactive_mode,
            commands::set_proactive_mode,
            // Identity & Ambient commands (Feature 1 & Feature 2)
            commands::get_companion_name,
            commands::set_companion_name,
            commands::get_user_name,
            commands::set_user_name,
            commands::generate_ambient_message,
            commands::set_window_layout_state,
            commands::start_window_drag,
            // Avatar commands (Phase 3-A) [DR-036]
            commands::get_active_avatar,
            commands::set_active_avatar,
            commands::list_avatars,
            commands::get_avatar_image,
            // Plugin commands (Phase 3-D) [DR-039 through DR-044]
            commands::get_developer_mode,
            commands::set_developer_mode,
            commands::list_plugins,
            commands::approve_plugin_key,
            commands::remove_plugin,
            commands::call_plugin_tool,
        ])
        .run(tauri::generate_context!())
        .expect("Failed to start OpenMate");
}

/// Initialize all engines and return an `AppState`.
///
/// Order matters:
/// 1. Open database at runtime path (needed by PermissionEngine and MemoryEngine)
/// 2. Run migrations
/// 3. Initialize PermissionEngine (security layer first)
/// 4. Initialize MemoryEngine
/// 5. Initialize AI Provider (needed by ContextEngine)
/// 6. Initialize ContextEngine, ModeEngine, ToolEngine, AvatarLoader, PluginLoader
/// 7. Compose AppState
async fn initialize_engines(
    db_path: std::path::PathBuf,
    app_handle: Option<tauri::AppHandle>,
) -> Result<AppState, crate::error::OpenMateError> {
    // ── 1. Open SQLite database at runtime path [DR-006] ─────────────────
    let db = engine::memory::Database::connect(&db_path).await?;
    crate::platform::keychain::init_db_fallback(Some(db_path.clone()));

    // ── 2. Run migrations ────────────────────────────────────────────────
    engine::memory::run_migrations(&db).await?;

    // Clone connection handles — tokio-rusqlite Connection is Arc-based.
    let db_for_permissions = db.clone();
    let db_for_trust = db.clone();

    // ── 3. Permission Engine (security first) ────────────────────────────
    let permission_engine = Arc::new(PermissionEngine::new(db_for_permissions).await?);

    // ── 4. Memory Engine ─────────────────────────────────────────────────
    let memory_engine = Arc::new(MemoryEngine::new(db));

    // ── 5. AI & Voice Providers ──────────────────────────────────────────
    let ai_provider = Arc::new(GeminiProvider::new());
    let voice_provider = Arc::new(crate::ai::voice::GroqVoiceProvider::new());

    // ── 6. Remaining engines ─────────────────────────────────────────────
    let mode_engine = Arc::new(ModeEngine::new(CompanionMode::Assistant));

    let context_engine = Arc::new(ContextEngine::new(
        Arc::clone(&permission_engine),
        Arc::clone(&ai_provider) as Arc<dyn crate::ai::provider::AIProvider>,
    ));
    context_engine.set_memory_engine(Arc::clone(&memory_engine)).await;
    if let Some(ref handle) = app_handle {
        context_engine.set_app_handle(handle.clone()).await;
    }
    context_engine.start_monitor(2000).await;

    let tool_engine = Arc::new(ToolEngine::new(Arc::clone(&permission_engine)));

    // Resolve avatars directory: default to `avatars/` in project / executable dir
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let mut avatars_dir = cwd.join("avatars");
    let mut plugins_dir = cwd.join("plugins");
    if !avatars_dir.exists() && cwd.ends_with("src-tauri") {
        if let Some(parent) = cwd.parent() {
            let parent_avatars = parent.join("avatars");
            if parent_avatars.exists() {
                avatars_dir = parent_avatars;
            }
            let parent_plugins = parent.join("plugins");
            if parent_plugins.exists() {
                plugins_dir = parent_plugins;
            }
        }
    }
    if !avatars_dir.exists() {
        let _ = std::fs::create_dir_all(&avatars_dir);
    }
    if !plugins_dir.exists() {
        let _ = std::fs::create_dir_all(&plugins_dir);
    }

    let avatar_loader = Arc::new(crate::engine::avatar::AvatarLoader::new(avatars_dir));

    // Initialize Plugin Trust Registry & Loader
    let trust_registry = match crate::engine::plugin::TrustRegistry::load_with_db(&db_for_trust).await {
        Ok(t) => t,
        Err(_) => crate::engine::plugin::TrustRegistry::load_bundled()
            .map_err(|e| crate::error::OpenMateError::Internal(e.to_string()))?,
    };
    let plugin_trust = Arc::new(tokio::sync::RwLock::new(trust_registry));
    let plugin_loader = Arc::new(crate::engine::plugin::PluginLoader::new(
        plugins_dir,
        Arc::clone(&plugin_trust),
    ));

    // ── 7. Compose state ─────────────────────────────────────────────────
    let mut state = AppState::new(
        permission_engine,
        memory_engine,
        mode_engine,
        context_engine,
        tool_engine,
        avatar_loader,
        plugin_loader,
        plugin_trust,
        ai_provider,
        voice_provider,
    );

    if let Some(handle) = app_handle {
        state = state.with_handle(handle);
    }

    Ok(state)
}
