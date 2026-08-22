//! # Plugin Trust Registry
//!
//! Manages bundled, community-synced, user-approved, and revoked plugin author keys. [DR-041]
//!
//! ## Critical Security Invariant:
//! **Revoked keys are ALWAYS blocked** — even if the user previously approved them. Revoked status
//! overrides everything.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;
use std::time::Duration;
use thiserror::Error;
use tokio_rusqlite::Connection;
use tracing::{info, warn};

const BUNDLED_TRUST_STORE_JSON: &str = include_str!("../../../../assets/trusted_authors.json");
const TRUST_REGISTRY_SYNC_URL: &str =
    "https://raw.githubusercontent.com/openmate/trust-registry/main/trusted_authors.json";

#[derive(Debug, Error)]
pub enum TrustError {
    #[error("Bundled trust store not found or corrupt")]
    BundledStoreNotFound,

    #[error("Failed to parse trust store: {0}")]
    ParseError(String),

    #[error("Network sync error: {0}")]
    NetworkError(String),

    #[error("Database error: {0}")]
    DatabaseError(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustLevel {
    Builtin,      // Official OpenMate core author
    Community,    // In bundled or synced trusted registry
    UserApproved, // Manually approved by user in Developer Mode
    Unknown,      // Not in registry
    Revoked,      // Explicitly revoked
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorEntry {
    pub pubkey: String,
    pub name: String,
    pub added: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustStore {
    pub version: String,
    #[serde(default)]
    pub authors: Vec<AuthorEntry>,
    #[serde(default)]
    pub revoked: Vec<String>,
}

pub struct TrustRegistry {
    bundled: TrustStore,
    user_approved: HashSet<String>,
    revoked: HashSet<String>,
}

impl TrustRegistry {
    /// Load the bundled trust store from embedded assets.
    pub fn load_bundled() -> Result<Self, TrustError> {
        let store: TrustStore = serde_json::from_str(BUNDLED_TRUST_STORE_JSON)
            .map_err(|e| TrustError::ParseError(e.to_string()))?;

        let mut revoked_set = HashSet::new();
        for r in &store.revoked {
            revoked_set.insert(r.trim().to_lowercase());
        }

        Ok(Self {
            bundled: store,
            user_approved: HashSet::new(),
            revoked: revoked_set,
        })
    }

    /// Load bundled store and hydrate user-approved keys from SQLite database.
    pub async fn load_with_db(db: &Connection) -> Result<Self, TrustError> {
        let mut registry = Self::load_bundled()?;
        registry.load_user_keys(db).await?;
        Ok(registry)
    }

    /// Load user-approved keys from SQLite `app_config` table.
    pub async fn load_user_keys(&mut self, db: &Connection) -> Result<(), TrustError> {
        let raw_val: Option<String> = db
            .call(|conn| {
                let mut stmt = conn
                    .prepare("SELECT value FROM app_config WHERE key = 'user_approved_plugin_keys'")
                    .map_err(|e| tokio_rusqlite::Error::Other(e.into()))?;
                let mut rows = stmt
                    .query([])
                    .map_err(|e| tokio_rusqlite::Error::Other(e.into()))?;
                if let Some(row) = rows.next().map_err(|e| tokio_rusqlite::Error::Other(e.into()))? {
                    let val: String = row.get(0).map_err(|e| tokio_rusqlite::Error::Other(e.into()))?;
                    Ok(Some(val))
                } else {
                    Ok(None)
                }
            })
            .await
            .map_err(|e| TrustError::DatabaseError(e.to_string()))?;

        if let Some(json_str) = raw_val {
            if let Ok(keys) = serde_json::from_str::<Vec<String>>(&json_str) {
                for k in keys {
                    self.user_approved.insert(k.trim().to_lowercase());
                }
            }
        }

        Ok(())
    }

    /// Persist user-approved keys to SQLite `app_config` table.
    pub async fn save_user_keys(&self, db: &Connection) -> Result<(), TrustError> {
        let keys: Vec<String> = self.user_approved.iter().cloned().collect();
        let json_str = serde_json::to_string(&keys)
            .map_err(|e| TrustError::ParseError(e.to_string()))?;

        db.call(move |conn| {
            conn.execute(
                "INSERT INTO app_config (key, value) VALUES ('user_approved_plugin_keys', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                rusqlite::params![json_str],
            )
            .map_err(|e| tokio_rusqlite::Error::Other(e.into()))?;
            Ok(())
        })
        .await
        .map_err(|e| TrustError::DatabaseError(e.to_string()))?;

        Ok(())
    }

    /// Check the trust level of a given author public key.
    /// Invariant: Revoked keys always return `TrustLevel::Revoked`.
    pub fn is_trusted(&self, pubkey: &str) -> TrustLevel {
        let normalized = pubkey.trim().to_lowercase();

        // 1. Revocation check always takes priority
        if self.revoked.contains(&normalized) {
            return TrustLevel::Revoked;
        }

        // 2. Check bundled / synced community author registry
        for author in &self.bundled.authors {
            if author.pubkey.trim().eq_ignore_ascii_case(&normalized) {
                if author.name.to_lowercase().contains("openmate") {
                    return TrustLevel::Builtin;
                }
                return TrustLevel::Community;
            }
        }

        // 3. Check user manually approved whitelist
        if self.user_approved.contains(&normalized) {
            return TrustLevel::UserApproved;
        }

        // 4. Default to Unknown (Unsigned/Untrusted)
        TrustLevel::Unknown
    }

    /// Add a user-approved author public key (Developer Mode).
    pub fn approve_user_key(&mut self, pubkey: String) {
        let normalized = pubkey.trim().to_lowercase();
        self.user_approved.insert(normalized);
    }

    /// Remove a user-approved author key.
    pub fn revoke_user_key(&mut self, pubkey: &str) {
        let normalized = pubkey.trim().to_lowercase();
        self.user_approved.remove(&normalized);
    }

    /// Fetch updated trust registry from online source with 5-second timeout and offline fallback.
    pub async fn sync_online(&mut self) -> Result<(), TrustError> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .map_err(|e| TrustError::NetworkError(e.to_string()))?;

        let res = match client.get(TRUST_REGISTRY_SYNC_URL).send().await {
            Ok(r) => r,
            Err(e) => {
                warn!("[TrustRegistry] Failed to sync online registry (using offline bundled fallback): {}", e);
                return Err(TrustError::NetworkError(e.to_string()));
            }
        };

        if !res.status().is_success() {
            warn!("[TrustRegistry] Online registry returned HTTP status {}", res.status());
            return Err(TrustError::NetworkError(format!("HTTP {}", res.status())));
        }

        let body = res
            .text()
            .await
            .map_err(|e| TrustError::NetworkError(e.to_string()))?;

        let new_store: TrustStore =
            serde_json::from_str(&body).map_err(|e| TrustError::ParseError(e.to_string()))?;

        for r in &new_store.revoked {
            self.revoked.insert(r.trim().to_lowercase());
        }

        self.bundled = new_store;
        info!("[TrustRegistry] Successfully synchronized trust store with remote registry");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trust_registry_builtin() {
        let registry = TrustRegistry::load_bundled().unwrap();
        let level = registry.is_trusted("ed25519:3b6f2c7d9e1a4f5b8c0d2e3f4a5b6c7d8e9f0a1b2c3d4e5f6a7b8c9d0e1f2a3b");
        assert_eq!(level, TrustLevel::Builtin);
    }

    #[test]
    fn test_trust_registry_community() {
        let mut registry = TrustRegistry::load_bundled().unwrap();
        registry.bundled.authors.push(AuthorEntry {
            pubkey: "ed25519:1111111111111111111111111111111111111111111111111111111111111111".to_string(),
            name: "Community Dev".to_string(),
            added: "2026-08-21".to_string(),
        });

        let level = registry.is_trusted("ed25519:1111111111111111111111111111111111111111111111111111111111111111");
        assert_eq!(level, TrustLevel::Community);
    }

    #[test]
    fn test_trust_registry_user_approved() {
        let mut registry = TrustRegistry::load_bundled().unwrap();
        let custom_key = "ed25519:2222222222222222222222222222222222222222222222222222222222222222";
        
        assert_eq!(registry.is_trusted(custom_key), TrustLevel::Unknown);

        registry.approve_user_key(custom_key.to_string());
        assert_eq!(registry.is_trusted(custom_key), TrustLevel::UserApproved);
    }

    #[test]
    fn test_trust_registry_revoked_always_blocked() {
        let mut registry = TrustRegistry::load_bundled().unwrap();
        let revoked_key = "ed25519:deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef";

        // Even if user attempts to approve a revoked key, Revoked ALWAYS wins!
        registry.approve_user_key(revoked_key.to_string());
        assert_eq!(
            registry.is_trusted(revoked_key),
            TrustLevel::Revoked,
            "Revoked key must ALWAYS return TrustLevel::Revoked"
        );
    }
}
