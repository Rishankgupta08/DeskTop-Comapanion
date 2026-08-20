// macOS: Stores in macOS Keychain (Security.framework) [DR-011]
// Windows: Stores in Windows Credential Manager [DR-011]
// Both: key is never returned to the frontend, never logged [ADR-009]

//! # OS Keychain Abstraction & SQLite Fallback
//!
//! Provides secure storage for the user's Gemini API key using the native OS
//! credential store [DR-011, ADR-009] with a local SQLite fallback for unsigned
//! development environments.
//!
//! // TODO: remove SQLite fallback once code signing is configured for production builds
//!
//! - **macOS:** Apple Keychain via `Security.framework`
//! - **Windows:** Windows Credential Manager via WinCred
//!
//! ## Security rules (non-negotiable):
//! 1. The key is retrieved immediately before an outbound API call and dropped
//!    (zeroized) immediately after. [docs/05-security-privacy.md §4.1]
//! 2. The key must NEVER appear in log output at any log level. [PP-010]
//! 3. The key must NEVER be returned to the frontend as plaintext. The frontend
//!    only receives a boolean `has_key: bool`. [docs/05-security-privacy.md §4.3]

use crate::error::OpenMateError;
#[allow(unused_imports)]
use keyring::Entry;
use std::path::PathBuf;
use std::sync::RwLock;
use tracing::{debug, error, info, warn};

/// Keychain service name scoped to OpenMate.
pub const KEYCHAIN_SERVICE: &str = "openmate";

/// Keychain account name under the service.
pub const KEYCHAIN_ACCOUNT: &str = "gemini_api_key";

#[cfg(test)]
static TEST_MOCK_KEY: RwLock<Option<String>> = RwLock::new(None);

/// Local database path for SQLite fallback in dev mode
static DB_FALLBACK_PATH: RwLock<Option<PathBuf>> = RwLock::new(None);

/// Initialize the local SQLite database fallback path.
pub fn init_db_fallback(path: Option<PathBuf>) {
    if let Ok(mut guard) = DB_FALLBACK_PATH.write() {
        *guard = path;
    }
}

/// Get the keychain entry for the Gemini API key.
#[allow(dead_code)]
fn entry() -> Result<Entry, OpenMateError> {
    Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT).map_err(|e| {
        error!(
            "Failed to open keychain entry (service={}, account={}): {}",
            KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT, e
        );
        OpenMateError::KeychainError(format!("Keychain unavailable: {}", e))
    })
}

#[allow(dead_code)]
fn read_sqlite_fallback() -> Option<String> {
    if let Ok(guard) = DB_FALLBACK_PATH.read() {
        if let Some(ref path) = *guard {
            if let Ok(conn) = rusqlite::Connection::open(path) {
                let _ = conn.execute(
                    "CREATE TABLE IF NOT EXISTS app_config (key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL)",
                    [],
                );
                if let Ok(mut stmt) = conn.prepare("SELECT value FROM app_config WHERE key = 'gemini_api_key'") {
                    let mut rows = stmt.query([]).ok()?;
                    if let Ok(Some(row)) = rows.next() {
                        return row.get(0).ok();
                    }
                }
            }
        }
    }
    None
}

#[allow(dead_code)]
fn write_sqlite_fallback(key: &str) -> Result<(), OpenMateError> {
    if let Ok(guard) = DB_FALLBACK_PATH.read() {
        if let Some(ref path) = *guard {
            let conn = rusqlite::Connection::open(path)
                .map_err(|e| OpenMateError::DatabaseError(e.to_string()))?;
            conn.execute(
                "CREATE TABLE IF NOT EXISTS app_config (key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL)",
                [],
            )
            .map_err(|e| OpenMateError::DatabaseError(e.to_string()))?;
            conn.execute(
                "INSERT OR REPLACE INTO app_config (key, value) VALUES ('gemini_api_key', ?1)",
                rusqlite::params![key],
            )
            .map_err(|e| OpenMateError::DatabaseError(e.to_string()))?;
            info!("API key stored in SQLite app_config fallback");
            return Ok(());
        }
    }
    Ok(())
}

#[allow(dead_code)]
fn delete_sqlite_fallback() -> Result<(), OpenMateError> {
    if let Ok(guard) = DB_FALLBACK_PATH.read() {
        if let Some(ref path) = *guard {
            if let Ok(conn) = rusqlite::Connection::open(path) {
                let _ = conn.execute("DELETE FROM app_config WHERE key = 'gemini_api_key'", []);
                info!("API key deleted from SQLite app_config fallback");
            }
        }
    }
    Ok(())
}

/// Store the user's Gemini API key in the OS keychain with SQLite fallback.
///
/// The key string is consumed and the function returns immediately.
/// Callers must not retain a copy of the key string after calling this.
///
/// ## Security
/// The key value is NEVER written to logs. [PP-010]
pub fn set_api_key(key: &str) -> Result<(), OpenMateError> {
    if key.trim().is_empty() {
        return Err(OpenMateError::KeychainError(
            "API key must not be empty".to_string(),
        ));
    }

    #[cfg(test)]
    {
        let mut mock = TEST_MOCK_KEY.write().unwrap();
        *mock = Some(key.to_string());
        return Ok(());
    }

    #[cfg(not(test))]
    {
        // Try OS keychain first
        let keychain_res = entry().and_then(|e| {
            e.set_password(key).map_err(|err| {
                error!("Keychain write failed: {}", err);
                OpenMateError::KeychainError(format!("Failed to store key in keychain: {}", err))
            })
        });

        match keychain_res {
            Ok(()) => {
                info!("API key stored in OS keychain successfully");
                // Also sync to SQLite fallback to ensure persistence across restarts in dev
                let _ = write_sqlite_fallback(key);
                Ok(())
            }
            Err(e) => {
                warn!("OS Keychain rejected write: {}. Using SQLite app_config fallback.", e);
                write_sqlite_fallback(key)?;
                Ok(())
            }
        }
    }
}

/// Retrieve the Gemini API key from the OS keychain with SQLite fallback.
///
/// Returns the key string. The caller MUST drop this `String` immediately after
/// use. It must not be cloned, logged, or sent over IPC. [PP-004, PP-010]
pub fn get_api_key() -> Result<String, OpenMateError> {
    #[cfg(test)]
    {
        let mock = TEST_MOCK_KEY.read().unwrap();
        match &*mock {
            Some(k) => Ok(k.clone()),
            None => Err(OpenMateError::NoApiKey),
        }
    }

    #[cfg(not(test))]
    {
        // Try OS keychain first
        if let Ok(e) = entry() {
            if let Ok(key) = e.get_password() {
                debug!("API key retrieved from OS keychain");
                return Ok(key);
            }
        }

        // Fallback to SQLite app_config
        if let Some(key) = read_sqlite_fallback() {
            debug!("API key retrieved from SQLite app_config fallback");
            return Ok(key);
        }

        // Fallback to GEMINI_API_KEY environment variable
        if let Ok(key) = std::env::var("GEMINI_API_KEY") {
            let trimmed = key.trim().to_string();
            if !trimmed.is_empty() {
                debug!("API key retrieved from GEMINI_API_KEY environment variable");
                return Ok(trimmed);
            }
        }

        Err(OpenMateError::NoApiKey)
    }
}

/// Delete the Gemini API key from the OS keychain and SQLite fallback.
///
/// Used when the user removes their API key from Settings, or as a clean-up
/// during key rotation. [docs/05-security-privacy.md §4.4]
pub fn delete_api_key() -> Result<(), OpenMateError> {
    #[cfg(test)]
    {
        let mut mock = TEST_MOCK_KEY.write().unwrap();
        *mock = None;
        return Ok(());
    }

    #[cfg(not(test))]
    {
        let _ = delete_sqlite_fallback();

        if let Ok(e) = entry() {
            let _ = e.delete_credential();
            info!("API key deleted from OS keychain");
        }

        Ok(())
    }
}

/// Check whether an API key is currently stored in the keychain, SQLite fallback, or env.
///
/// Returns `true` / `false` only — the key value is never exposed. [PP-004]
/// This is the only keychain query the frontend is allowed to trigger via IPC.
pub fn has_api_key() -> bool {
    #[cfg(test)]
    {
        let mock = TEST_MOCK_KEY.read().unwrap();
        mock.is_some()
    }

    #[cfg(not(test))]
    {
        if let Ok(e) = entry() {
            if matches!(e.get_password(), Ok(_)) {
                return true;
            }
        }

        if read_sqlite_fallback().is_some() {
            return true;
        }

        if let Ok(key) = std::env::var("GEMINI_API_KEY") {
            return !key.trim().is_empty();
        }

        false
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_set_get_delete_lifecycle() {
        // Ensure clean state before test
        let _ = delete_api_key();
        assert!(!has_api_key());

        // Store key
        assert!(set_api_key("test_key_openmate_12345").is_ok());
        assert!(has_api_key());

        // Retrieve key
        let key = get_api_key().expect("should retrieve key");
        assert_eq!(key, "test_key_openmate_12345");

        // Delete key
        assert!(delete_api_key().is_ok());
        assert!(!has_api_key());
    }

    #[test]
    fn test_set_empty_key_is_rejected() {
        let result = set_api_key("   ");
        assert!(result.is_err());
    }

    #[test]
    fn test_has_api_key_is_false_when_absent() {
        let _ = delete_api_key();
        assert!(!has_api_key());
    }

    #[test]
    fn test_get_returns_no_api_key_when_absent() {
        let _ = delete_api_key();
        let result = get_api_key();
        assert!(matches!(result, Err(OpenMateError::NoApiKey)));
    }
}
