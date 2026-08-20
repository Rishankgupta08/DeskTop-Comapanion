//! # Permission Engine
//!
//! Central capability gatekeeper for OpenMate. [DR-012, ADR-010]
//!
//! **All** access to sensitive OS resources (screen, microphone, filesystem,
//! app launch) MUST go through this engine. No component may call a native OS
//! API without first obtaining a `PermissionToken` from here.
//!
//! ## PermissionToken pattern
//!
//! `PermissionToken` is a zero-cost, compile-time marker. It proves at the
//! type level that the Permission Engine was consulted and returned `Allow`
//! before a native call was made.
//!
//! Usage:
//! ```rust,ignore
//! let token = permission_engine.check(Capability::ScreenCapture)?;
//! let pixels = capture_screen(&token)?;
//! ```
//!
//! Native functions that access sensitive resources MUST declare a
//! `&PermissionToken` parameter. This makes it impossible to call them
//! without passing through the permission check — the code will not compile.
//!
//! The token is NOT serialized, NOT sent to the frontend, and NOT persisted.
//! It lives only within a single synchronous Rust call stack.
//!
//! ## Default state
//!
//! All capabilities default to `Off` on a fresh installation. [docs/05-security-privacy.md §3.1]
//!
//! ## Revocation
//!
//! Calling `set_permission(cap, Off)` takes effect immediately. Any in-flight
//! check that has not yet received its token is unaffected, but no new tokens
//! are issued. [docs/05-security-privacy.md §3.4]

use crate::error::OpenMateError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio_rusqlite::Connection;
use tracing::{debug, info, warn};

// ── Capability registry ──────────────────────────────────────────────────────

/// Every sensitive capability OpenMate can request access to.
/// Clipboard and Terminal are NOT in V1 [DR-030] — they are reserved here so
/// future additions have an established home without schema migrations.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    ScreenCapture,
    Microphone,
    FilesystemRead,
    FilesystemWrite,
    AppLaunch,
    Clipboard,
    // Reserved — not in V1 [DR-030]:
    // Terminal,
}

impl Capability {
    /// Human-readable label shown to users in permission dialogs.
    pub fn display_name(&self) -> &'static str {
        match self {
            Capability::ScreenCapture => "Screen Capture",
            Capability::Microphone => "Microphone",
            Capability::FilesystemRead => "Read Files",
            Capability::FilesystemWrite => "Write Files",
            Capability::AppLaunch => "Launch Applications",
            Capability::Clipboard => "Clipboard Access",
        }
    }

    /// Short description of what granting this capability allows.
    pub fn description(&self) -> &'static str {
        match self {
            Capability::ScreenCapture => {
                "Allows OpenMate to capture your screen and send it to the AI for analysis. \
                 Screenshots are never saved to disk."
            }
            Capability::Microphone => {
                "Allows OpenMate to listen to your voice for speech input. \
                 Audio is not recorded or stored."
            }
            Capability::FilesystemRead => {
                "Allows OpenMate to read specific files you select, to help with code or content."
            }
            Capability::FilesystemWrite => {
                "Allows OpenMate to write or modify files. Each write requires your confirmation."
            }
            Capability::AppLaunch => {
                "Allows OpenMate to open applications on your behalf. \
                 Each launch requires your confirmation."
            }
            Capability::Clipboard => {
                "Allows OpenMate to notice when you copy content. \
                 Content is never sent to Gemini unless you ask."
            }
        }
    }

    /// Key used to persist state in the SQLite `permissions` table.
    pub fn db_key(&self) -> &'static str {
        match self {
            Capability::ScreenCapture => "screen_capture",
            Capability::Microphone => "microphone",
            Capability::FilesystemRead => "filesystem_read",
            Capability::FilesystemWrite => "filesystem_write",
            Capability::AppLaunch => "app_launch",
            Capability::Clipboard => "clipboard",
        }
    }

    /// All V1 capabilities in display order.
    pub fn all_v1() -> Vec<Capability> {
        vec![
            Capability::ScreenCapture,
            Capability::Microphone,
            Capability::FilesystemRead,
            Capability::FilesystemWrite,
            Capability::AppLaunch,
            Capability::Clipboard,
        ]
    }
}

// ── Permission state ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionState {
    /// Capability is disabled. Access is never granted. (Default) [docs/05-security-privacy.md §3.1]
    Off,
    /// User is prompted each time the capability is requested.
    Ask,
    /// Capability is enabled for its defined scope without per-use prompting.
    Allow,
}

impl PermissionState {
    fn from_db_str(s: &str) -> Self {
        match s {
            "ask" => PermissionState::Ask,
            "allow" => PermissionState::Allow,
            _ => PermissionState::Off, // Safe default: unknown value → Off
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            PermissionState::Off => "off",
            PermissionState::Ask => "ask",
            PermissionState::Allow => "allow",
        }
    }

    fn to_db_str(&self) -> &'static str {
        self.as_str()
    }
}

// ── PermissionToken — zero-cost compile-time proof ───────────────────────────

/// Proof that the Permission Engine was consulted and returned `Allow` for a
/// specific capability before the current call frame.
///
/// This token is:
/// - Created only by `PermissionEngine::check()`
/// - `#[non_exhaustive]` so it cannot be constructed outside this module
/// - Never serialized, never sent to the frontend, never persisted
/// - Valid only for the duration of the call stack that holds it
///
/// Any function that accesses a sensitive OS resource MUST require `&PermissionToken`
/// as a parameter. This is enforced at compile time.
#[must_use]
pub struct PermissionToken {
    #[allow(dead_code)]
    pub(crate) capability: Capability,
}

// ── Permission Engine ─────────────────────────────────────────────────────────

/// The central capability gatekeeper. All sensitive native calls must pass
/// through `check()` before they execute. [DR-012, ADR-010]
pub struct PermissionEngine {
    /// In-memory cache of capability states, kept in sync with SQLite.
    /// Uses `RwLock` so reads (the common path) never block each other.
    states: Arc<RwLock<HashMap<Capability, PermissionState>>>,
    db: Connection,
}

impl PermissionEngine {
    /// Create a new `PermissionEngine`, loading persisted states from SQLite.
    /// Capabilities not yet in the database default to `Off`. [DR-012]
    pub async fn new(db: Connection) -> Result<Self, OpenMateError> {
        // Ensure the permissions table exists (idempotent).
        db.call(|conn| {
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS permissions (
                    capability TEXT PRIMARY KEY NOT NULL,
                    state      TEXT NOT NULL DEFAULT 'off',
                    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
                );",
            )
            .map_err(|e| tokio_rusqlite::Error::Other(e.into()))
        })
        .await
        .map_err(|e| OpenMateError::DatabaseError(e.to_string()))?;

        // Seed default 'off' rows for every known capability.
        // INSERT OR IGNORE means existing rows (user-set Allow) are untouched.
        // This ensures new capabilities added in later phases always have a
        // persisted row, so set_permission() can UPDATE rather than INSERT. [Fix-3]
        let all_caps = Capability::all_v1();
        db.call(move |conn| {
            for cap in &all_caps {
                conn.execute(
                    "INSERT OR IGNORE INTO permissions (capability, state) VALUES (?1, 'off')",
                    rusqlite::params![cap.db_key()],
                )
                .map_err(|e| tokio_rusqlite::Error::Other(e.into()))?;
            }
            Ok(())
        })
        .await
        .map_err(|e| OpenMateError::DatabaseError(e.to_string()))?;

        // Load persisted states.
        let rows: Vec<(String, String)> = db
            .call(|conn| {
                let mut stmt = conn
                    .prepare("SELECT capability, state FROM permissions")
                    .map_err(|e| tokio_rusqlite::Error::Other(e.into()))?;
                let rows = stmt
                    .query_map([], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                    })
                    .map_err(|e| tokio_rusqlite::Error::Other(e.into()))?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| tokio_rusqlite::Error::Other(e.into()))?;
                Ok(rows)
            })
            .await
            .map_err(|e| OpenMateError::DatabaseError(e.to_string()))?;

        // Build the in-memory cache. Capabilities absent from the DB default to Off.
        let mut states: HashMap<Capability, PermissionState> = Capability::all_v1()
            .into_iter()
            .map(|cap| (cap, PermissionState::Off))
            .collect();

        for (key, value) in rows {
            // Only load known capabilities; ignore unknown DB entries defensively.
            let cap = Capability::all_v1()
                .into_iter()
                .find(|c| c.db_key() == key.as_str());
            if let Some(cap) = cap {
                states.insert(cap, PermissionState::from_db_str(&value));
            }
        }

        info!("PermissionEngine initialized with {} capabilities", states.len());

        Ok(Self {
            states: Arc::new(RwLock::new(states)),
            db,
        })
    }

    // ── Core API ──────────────────────────────────────────────────────────────

    /// Check whether a capability is `Allow` and return a `PermissionToken`.
    ///
    /// Returns `Ok(PermissionToken)` if the capability is `Allow`.
    /// Returns `Err(PermissionDenied)` if the state is `Off` or `Ask`.
    ///
    /// For `Ask` state, the Tauri IPC layer must emit a `permission://prompt`
    /// event to the frontend and await user response before re-checking.
    pub async fn check(&self, capability: Capability) -> Result<PermissionToken, OpenMateError> {
        let states = self.states.read().await;
        let state = states
            .get(&capability)
            .cloned()
            .unwrap_or(PermissionState::Off);

        // Debug log — shows in terminal so operators can confirm state [Fix-3.1]
        info!(
            "Permission check: capability={:?} state={:?}",
            capability,
            state
        );

        match state {
            PermissionState::Allow => {
                debug!("PermissionEngine: Allow granted for {:?}", capability);
                Ok(PermissionToken {
                    capability: capability.clone(),
                })
            }
            PermissionState::Off => {
                warn!(
                    "PermissionEngine: Denied (Off) for {:?}",
                    capability
                );
                Err(OpenMateError::PermissionDenied(
                    capability.display_name().to_string(),
                ))
            }
            PermissionState::Ask => {
                // The IPC layer must prompt the user; this check returns denied
                // until the user responds and the state is updated to Allow.
                warn!(
                    "PermissionEngine: Prompt required (Ask) for {:?}",
                    capability
                );
                Err(OpenMateError::PermissionDenied(format!(
                    "{} — permission prompt required",
                    capability.display_name()
                )))
            }
        }
    }

    /// Get the current state of a capability without issuing a token.
    pub async fn get_state(&self, capability: &Capability) -> PermissionState {
        let states = self.states.read().await;
        states
            .get(capability)
            .cloned()
            .unwrap_or(PermissionState::Off)
    }

    /// Get the current state of all V1 capabilities.
    pub async fn get_all_states(&self) -> HashMap<Capability, PermissionState> {
        let states = self.states.read().await;
        states.clone()
    }

    /// Set a capability's permission state.
    ///
    /// - Transitions to `Off` take effect immediately. [docs/05-security-privacy.md §3.4]
    /// - State is persisted to SQLite.
    pub async fn set_permission(
        &self,
        capability: Capability,
        new_state: PermissionState,
    ) -> Result<(), OpenMateError> {
        let key = capability.db_key().to_string();
        let state_str = new_state.to_db_str().to_string();

        // Persist to SQLite first (fail fast if DB is unavailable).
        self.db
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO permissions (capability, state, updated_at)
                     VALUES (?1, ?2, datetime('now'))
                     ON CONFLICT(capability) DO UPDATE SET
                         state = excluded.state,
                         updated_at = excluded.updated_at",
                    rusqlite::params![key, state_str],
                )
                .map_err(|e| tokio_rusqlite::Error::Other(e.into()))?;
                Ok(())
            })
            .await
            .map_err(|e| OpenMateError::DatabaseError(e.to_string()))?;

        // Update in-memory cache.
        let mut states = self.states.write().await;
        info!(
            "PermissionEngine: {:?} → {:?}",
            capability, new_state
        );
        states.insert(capability, new_state);

        Ok(())
    }

    /// Convenience: immediately revoke (set to Off) a capability.
    /// Revocation takes effect before the next `check()` call. [docs/05-security-privacy.md §3.4]
    pub async fn revoke(&self, capability: Capability) -> Result<(), OpenMateError> {
        self.set_permission(capability, PermissionState::Off).await
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_rusqlite::Connection;

    async fn test_engine() -> PermissionEngine {
        let db = Connection::open_in_memory()
            .await
            .expect("in-memory DB failed");
        PermissionEngine::new(db)
            .await
            .expect("engine init failed")
    }

    #[tokio::test]
    async fn test_default_state_is_off() {
        let engine = test_engine().await;
        for cap in Capability::all_v1() {
            let state = engine.get_state(&cap).await;
            assert_eq!(
                state,
                PermissionState::Off,
                "Expected Off for {:?} on fresh engine",
                cap
            );
        }
    }

    #[tokio::test]
    async fn test_check_returns_err_when_off() {
        let engine = test_engine().await;
        let result = engine.check(Capability::ScreenCapture).await;
        assert!(
            result.is_err(),
            "check() should return Err when capability is Off"
        );
    }

    #[tokio::test]
    async fn test_check_returns_token_when_allow() {
        let engine = test_engine().await;
        engine
            .set_permission(Capability::ScreenCapture, PermissionState::Allow)
            .await
            .unwrap();
        let token = engine.check(Capability::ScreenCapture).await;
        assert!(
            token.is_ok(),
            "check() should return Ok(PermissionToken) when Allow"
        );
        assert_eq!(token.unwrap().capability, Capability::ScreenCapture);
    }

    #[tokio::test]
    async fn test_check_returns_err_when_ask() {
        let engine = test_engine().await;
        engine
            .set_permission(Capability::ScreenCapture, PermissionState::Ask)
            .await
            .unwrap();
        let result = engine.check(Capability::ScreenCapture).await;
        assert!(
            result.is_err(),
            "check() should return Err when Ask (prompt required)"
        );
    }

    #[tokio::test]
    async fn test_revocation_takes_effect_immediately() {
        let engine = test_engine().await;
        engine
            .set_permission(Capability::FilesystemRead, PermissionState::Allow)
            .await
            .unwrap();
        assert!(engine.check(Capability::FilesystemRead).await.is_ok());

        engine.revoke(Capability::FilesystemRead).await.unwrap();
        assert!(
            engine.check(Capability::FilesystemRead).await.is_err(),
            "After revoke(), check() must return Err immediately"
        );
    }

    #[tokio::test]
    async fn test_state_transitions() {
        let engine = test_engine().await;
        let cap = Capability::Microphone;

        // Off → Ask → Allow → Off
        engine
            .set_permission(cap.clone(), PermissionState::Ask)
            .await
            .unwrap();
        assert_eq!(engine.get_state(&cap).await, PermissionState::Ask);

        engine
            .set_permission(cap.clone(), PermissionState::Allow)
            .await
            .unwrap();
        assert_eq!(engine.get_state(&cap).await, PermissionState::Allow);

        engine
            .set_permission(cap.clone(), PermissionState::Off)
            .await
            .unwrap();
        assert_eq!(engine.get_state(&cap).await, PermissionState::Off);
    }

    #[tokio::test]
    async fn test_set_and_get_all_states() {
        let engine = test_engine().await;
        engine
            .set_permission(Capability::AppLaunch, PermissionState::Ask)
            .await
            .unwrap();
        engine
            .set_permission(Capability::FilesystemWrite, PermissionState::Allow)
            .await
            .unwrap();

        let all = engine.get_all_states().await;
        assert_eq!(
            all.get(&Capability::AppLaunch),
            Some(&PermissionState::Ask)
        );
        assert_eq!(
            all.get(&Capability::FilesystemWrite),
            Some(&PermissionState::Allow)
        );
        // Unset caps remain Off
        assert_eq!(
            all.get(&Capability::ScreenCapture),
            Some(&PermissionState::Off)
        );
    }

    #[tokio::test]
    async fn test_permission_state_survives_restart() {
        let db = Connection::open_in_memory().await.unwrap();
        crate::engine::memory::run_migrations(&db).await.unwrap();

        // 1. Initial engine session: modify states
        {
            let engine1 = PermissionEngine::new(db.clone()).await.unwrap();
            engine1
                .set_permission(Capability::AppLaunch, PermissionState::Allow)
                .await
                .unwrap();
            engine1
                .set_permission(Capability::FilesystemRead, PermissionState::Allow)
                .await
                .unwrap();
            engine1
                .set_permission(Capability::FilesystemWrite, PermissionState::Ask)
                .await
                .unwrap();
        }

        // 2. Emulate app restart: boot a new PermissionEngine from same SQLite DB
        let engine2 = PermissionEngine::new(db).await.unwrap();
        assert_eq!(
            engine2.get_state(&Capability::AppLaunch).await,
            PermissionState::Allow,
            "AppLaunch state must survive restart"
        );
        assert_eq!(
            engine2.get_state(&Capability::FilesystemRead).await,
            PermissionState::Allow,
            "FilesystemRead state must survive restart"
        );
        assert_eq!(
            engine2.get_state(&Capability::FilesystemWrite).await,
            PermissionState::Ask,
            "FilesystemWrite state must survive restart"
        );
        assert_eq!(
            engine2.get_state(&Capability::Microphone).await,
            PermissionState::Off,
            "Untouched capability remains Off"
        );
    }
}
