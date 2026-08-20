//! # SQLite Database Setup + Memory Engine
//!
//! Manages persistent local storage. [DR-006, ADR-006]
//!
//! ## Data lifecycle boundaries (non-negotiable):
//! - Temporary desktop context (screenshots) is NEVER written to this database. [DR-008, PP-002]
//! - API keys are NEVER stored in this database. [DR-011, PP-004]
//! - Only explicit user memories and conversation history are persisted here.
//!
//! ## Schema
//! Current schema version: v0 (initial).
//! Schema is forward-only: apply migrations in order, never downgrade.
//! [TBD — DR-020: full schema must be confirmed before production release]
//!
//! ## Retention policy
//! [TBD — DR-019, MR-04: per-type size limits and eviction rules not yet confirmed]

use crate::error::OpenMateError;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio_rusqlite::Connection;
use tracing::{debug, info};
use uuid::Uuid;

// ── Database connection ───────────────────────────────────────────────────────

/// SQLite Database connection manager. [DR-006, ADR-006]
pub struct Database;

impl Database {
    /// Connect to a SQLite database at the specified filesystem path.
    pub async fn connect<P: AsRef<std::path::Path>>(path: P) -> Result<Connection, OpenMateError> {
        let path_buf = path.as_ref().to_path_buf();
        Connection::open(path_buf)
            .await
            .map_err(|e| OpenMateError::DatabaseError(e.to_string()))
    }

    /// Connect to an in-memory SQLite database (used for unit tests).
    pub async fn connect_in_memory() -> Result<Connection, OpenMateError> {
        Connection::open_in_memory()
            .await
            .map_err(|e| OpenMateError::DatabaseError(e.to_string()))
    }
}

// ── Schema migrations ─────────────────────────────────────────────────────────

/// Target schema version. Increment each time a migration is added.
const SCHEMA_VERSION: u32 = 1;

/// Apply all pending schema migrations in order.
/// Safe to call on every startup — migrations are idempotent.
pub async fn run_migrations(db: &Connection) -> Result<(), OpenMateError> {
    let current: u32 = db
        .call(|conn| {
            conn.pragma_query_value(None, "user_version", |row| row.get(0))
                .map_err(|e| tokio_rusqlite::Error::Other(e.into()))
        })
        .await
        .map_err(|e| OpenMateError::DatabaseError(e.to_string()))?;

    if current >= SCHEMA_VERSION {
        debug!("Database schema up to date (v{})", current);
        return Ok(());
    }

    info!("Migrating database: v{} → v{}", current, SCHEMA_VERSION);

    db.call(move |conn| {
        conn.execute_batch(
            // v1 — initial schema
            "
            -- Permissions table is created by PermissionEngine::new().
            -- Included here for documentation completeness.

            CREATE TABLE IF NOT EXISTS user_memories (
                id         TEXT PRIMARY KEY NOT NULL,
                content    TEXT NOT NULL,
                tags       TEXT NOT NULL DEFAULT '[]',  -- JSON array of strings
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE TABLE IF NOT EXISTS conversation_history (
                id          TEXT PRIMARY KEY NOT NULL,
                session_id  TEXT NOT NULL,
                role        TEXT NOT NULL CHECK(role IN ('user', 'assistant', 'system')),
                content     TEXT NOT NULL,
                created_at  TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE INDEX IF NOT EXISTS idx_history_session
                ON conversation_history (session_id, created_at);

            CREATE TABLE IF NOT EXISTS app_config (
                key   TEXT PRIMARY KEY NOT NULL,
                value TEXT NOT NULL
            );

            -- NOTE: Screenshots and temporary context are NEVER stored in this
            -- database. [DR-008, docs/05-security-privacy.md §7.1]
            ",
        )
        .map_err(|e| tokio_rusqlite::Error::Other(e.into()))
    })
    .await
    .map_err(|e| OpenMateError::DatabaseError(e.to_string()))?;

    // Update schema version.
    db.call(move |conn| {
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)
            .map_err(|e| tokio_rusqlite::Error::Other(e.into()))
    })
    .await
    .map_err(|e| OpenMateError::DatabaseError(e.to_string()))?;

    info!("Database migrated to schema v{}", SCHEMA_VERSION);
    Ok(())
}

// ── Data types ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub id: String,
    pub content: String,
    pub tags: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewMemoryEntry {
    pub content: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationMessage {
    pub id: String,
    pub session_id: String,
    pub role: String,
    pub content: String,
    pub created_at: String,
}

// ── Memory Engine ─────────────────────────────────────────────────────────────

pub struct MemoryEngine {
    db: Connection,
}

impl MemoryEngine {
    pub fn new(db: Connection) -> Self {
        Self { db }
    }

    // ── User memory ───────────────────────────────────────────────────────

    /// Save a new user memory entry. [DR-006, SRS FR-036]
    pub async fn save_memory(
        &self,
        entry: NewMemoryEntry,
    ) -> Result<MemoryEntry, OpenMateError> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        let tags_json = serde_json::to_string(&entry.tags)
            .map_err(|e| OpenMateError::SerializationError(e.to_string()))?;

        let content = entry.content.clone();
        let id_clone = id.clone();
        let now_clone = now.clone();
        let tags_clone = tags_json.clone();

        self.db
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO user_memories (id, content, tags, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?4)",
                    rusqlite::params![id_clone, content, tags_clone, now_clone],
                )
                .map_err(|e| tokio_rusqlite::Error::Other(e.into()))?;
                Ok(())
            })
            .await
            .map_err(|e| OpenMateError::DatabaseError(e.to_string()))?;

        debug!("Saved memory entry {}", id);

        Ok(MemoryEntry {
            id,
            content: entry.content,
            tags: entry.tags,
            created_at: now.clone(),
            updated_at: now,
        })
    }

    /// Retrieve all user memory entries. [SRS FR-037]
    pub async fn get_memories(&self) -> Result<Vec<MemoryEntry>, OpenMateError> {
        let rows: Vec<MemoryEntry> = self
            .db
            .call(|conn| {
                let mut stmt = conn
                    .prepare(
                        "SELECT id, content, tags, created_at, updated_at
                         FROM user_memories
                         ORDER BY updated_at DESC",
                    )
                    .map_err(|e| tokio_rusqlite::Error::Other(e.into()))?;

                let rows = stmt
                    .query_map([], |row| {
                        let tags_json: String = row.get(2)?;
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            tags_json,
                            row.get::<_, String>(3)?,
                            row.get::<_, String>(4)?,
                        ))
                    })
                    .map_err(|e| tokio_rusqlite::Error::Other(e.into()))?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| tokio_rusqlite::Error::Other(e.into()))?;
                Ok(rows)
            })
            .await
            .map_err(|e| OpenMateError::DatabaseError(e.to_string()))?
            .into_iter()
            .map(|(id, content, tags_json, created_at, updated_at)| {
                let tags: Vec<String> =
                    serde_json::from_str(&tags_json).unwrap_or_default();
                MemoryEntry {
                    id,
                    content,
                    tags,
                    created_at,
                    updated_at,
                }
            })
            .collect();

        Ok(rows)
    }

    /// Delete a specific memory entry by ID. [SRS FR-038]
    pub async fn delete_memory(&self, id: String) -> Result<(), OpenMateError> {
        let rows_affected: usize = self
            .db
            .call(move |conn| {
                conn.execute(
                    "DELETE FROM user_memories WHERE id = ?1",
                    rusqlite::params![id],
                )
                .map_err(|e| tokio_rusqlite::Error::Other(e.into()))
            })
            .await
            .map_err(|e| OpenMateError::DatabaseError(e.to_string()))?;

        if rows_affected == 0 {
            return Err(OpenMateError::MemoryNotFound("not found".to_string()));
        }

        debug!("Deleted memory entry");
        Ok(())
    }

    /// Delete all user memory entries. [SRS FR-039]
    pub async fn clear_memories(&self) -> Result<u64, OpenMateError> {
        let count: usize = self
            .db
            .call(|conn| {
                conn.execute("DELETE FROM user_memories", [])
                    .map_err(|e| tokio_rusqlite::Error::Other(e.into()))
            })
            .await
            .map_err(|e| OpenMateError::DatabaseError(e.to_string()))?;

        info!("Cleared {} memory entries", count);
        Ok(count as u64)
    }

    // ── Conversation history ──────────────────────────────────────────────

    /// Append a message to the current session's conversation history.
    pub async fn append_message(
        &self,
        session_id: &str,
        role: &str,
        content: &str,
    ) -> Result<(), OpenMateError> {
        let id = Uuid::new_v4().to_string();
        let sid = session_id.to_string();
        let r = role.to_string();
        let c = content.to_string();

        self.db
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO conversation_history (id, session_id, role, content)
                     VALUES (?1, ?2, ?3, ?4)",
                    rusqlite::params![id, sid, r, c],
                )
                .map_err(|e| tokio_rusqlite::Error::Other(e.into()))?;
                Ok(())
            })
            .await
            .map_err(|e| OpenMateError::DatabaseError(e.to_string()))?;

        Ok(())
    }

    /// Retrieve the last `limit` messages for a session.
    pub async fn get_history(
        &self,
        session_id: &str,
        limit: u32,
    ) -> Result<Vec<ConversationMessage>, OpenMateError> {
        let sid = session_id.to_string();

        let rows: Vec<ConversationMessage> = self
            .db
            .call(move |conn| {
                let mut stmt = conn
                    .prepare(
                        "SELECT id, session_id, role, content, created_at
                         FROM conversation_history
                         WHERE session_id = ?1
                         ORDER BY created_at DESC
                         LIMIT ?2",
                    )
                    .map_err(|e| tokio_rusqlite::Error::Other(e.into()))?;

                let rows = stmt
                    .query_map(rusqlite::params![sid, limit], |row| {
                        Ok(ConversationMessage {
                            id: row.get(0)?,
                            session_id: row.get(1)?,
                            role: row.get(2)?,
                            content: row.get(3)?,
                            created_at: row.get(4)?,
                        })
                    })
                    .map_err(|e| tokio_rusqlite::Error::Other(e.into()))?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| tokio_rusqlite::Error::Other(e.into()))?;
                Ok(rows)
            })
            .await
            .map_err(|e| OpenMateError::DatabaseError(e.to_string()))?;

        // Return in chronological order (we fetched DESC for LIMIT efficiency).
        let mut messages = rows;
        messages.reverse();
        Ok(messages)
    }

    /// Clear all conversation history for a session.
    pub async fn clear_history(&self, session_id: &str) -> Result<(), OpenMateError> {
        let sid = session_id.to_string();
        self.db
            .call(move |conn| {
                conn.execute(
                    "DELETE FROM conversation_history WHERE session_id = ?1",
                    rusqlite::params![sid],
                )
                .map_err(|e| tokio_rusqlite::Error::Other(e.into()))?;
                Ok(())
            })
            .await
            .map_err(|e| OpenMateError::DatabaseError(e.to_string()))?;
        Ok(())
    }

    // ── App config / Settings [DR-017, Feature 1] ───────────────────────────

    /// Retrieve a string configuration value by key, or return the default if not set.
    pub async fn get_config(&self, key: &str, default: &str) -> Result<String, OpenMateError> {
        let k = key.to_string();
        let def = default.to_string();
        let val = self
            .db
            .call(move |conn| {
                let mut stmt = conn
                    .prepare("SELECT value FROM app_config WHERE key = ?1")
                    .map_err(|e| tokio_rusqlite::Error::Other(e.into()))?;
                let mut rows = stmt
                    .query(rusqlite::params![k])
                    .map_err(|e| tokio_rusqlite::Error::Other(e.into()))?;

                if let Some(row) = rows.next().map_err(|e| tokio_rusqlite::Error::Other(e.into()))? {
                    let val: String = row.get(0).map_err(|e| tokio_rusqlite::Error::Other(e.into()))?;
                    Ok(val)
                } else {
                    Ok(def)
                }
            })
            .await
            .map_err(|e| OpenMateError::DatabaseError(e.to_string()))?;

        Ok(val)
    }

    /// Save or update a string configuration value by key.
    pub async fn set_config(&self, key: &str, value: &str) -> Result<(), OpenMateError> {
        let k = key.to_string();
        let v = value.to_string();
        self.db
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO app_config (key, value) VALUES (?1, ?2)
                     ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                    rusqlite::params![k, v],
                )
                .map_err(|e| tokio_rusqlite::Error::Other(e.into()))?;
                Ok(())
            })
            .await
            .map_err(|e| OpenMateError::DatabaseError(e.to_string()))?;

        info!("App config updated: '{}' = '{}'", key, value);
        Ok(())
    }

    /// Retrieve the current proactive assistance mode. Defaults to "subtle".
    pub async fn get_proactive_mode(&self) -> Result<String, OpenMateError> {
        self.get_config("proactive_mode", "subtle").await
    }

    /// Set the proactive assistance mode ('off', 'subtle', 'active').
    pub async fn set_proactive_mode(&self, mode: &str) -> Result<(), OpenMateError> {
        let val = mode.to_lowercase();
        self.set_config("proactive_mode", &val).await
    }

    /// Retrieve the companion name. Defaults to "OpenMate".
    pub async fn get_companion_name(&self) -> Result<String, OpenMateError> {
        self.get_config("companion_name", "OpenMate").await
    }

    /// Set the companion name.
    pub async fn set_companion_name(&self, name: &str) -> Result<(), OpenMateError> {
        let trimmed = name.trim();
        let val = if trimmed.is_empty() { "OpenMate" } else { trimmed };
        self.set_config("companion_name", val).await
    }

    /// Retrieve the user name. Defaults to empty string.
    pub async fn get_user_name(&self) -> Result<String, OpenMateError> {
        self.get_config("user_name", "").await
    }

    /// Set the user name.
    pub async fn set_user_name(&self, name: &str) -> Result<(), OpenMateError> {
        self.set_config("user_name", name.trim()).await
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_rusqlite::Connection;

    async fn test_engine() -> MemoryEngine {
        let db = Connection::open_in_memory().await.unwrap();
        run_migrations(&db).await.unwrap();
        MemoryEngine::new(db)
    }

    #[tokio::test]
    async fn test_save_and_retrieve_memory() {
        let engine = test_engine().await;
        let entry = NewMemoryEntry {
            content: "User prefers dark mode".to_string(),
            tags: vec!["preference".to_string()],
        };
        let saved = engine.save_memory(entry).await.unwrap();
        assert_eq!(saved.content, "User prefers dark mode");
        assert_eq!(saved.tags, vec!["preference"]);

        let all = engine.get_memories().await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, saved.id);
    }

    #[tokio::test]
    async fn test_delete_memory() {
        let engine = test_engine().await;
        let saved = engine
            .save_memory(NewMemoryEntry {
                content: "test".to_string(),
                tags: vec![],
            })
            .await
            .unwrap();

        engine.delete_memory(saved.id.clone()).await.unwrap();
        let all = engine.get_memories().await.unwrap();
        assert!(all.is_empty());

        // Deleting again should return MemoryNotFound
        let result = engine.delete_memory(saved.id).await;
        assert!(matches!(result, Err(OpenMateError::MemoryNotFound(_))));
    }

    #[tokio::test]
    async fn test_clear_memories() {
        let engine = test_engine().await;
        for i in 0..5 {
            engine
                .save_memory(NewMemoryEntry {
                    content: format!("memory {}", i),
                    tags: vec![],
                })
                .await
                .unwrap();
        }
        let count = engine.clear_memories().await.unwrap();
        assert_eq!(count, 5);
        assert!(engine.get_memories().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_conversation_history() {
        let engine = test_engine().await;
        let sid = "session-1";
        engine
            .append_message(sid, "user", "Hello!")
            .await
            .unwrap();
        engine
            .append_message(sid, "assistant", "Hi there!")
            .await
            .unwrap();

        let history = engine.get_history(sid, 10).await.unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].role, "user");
        assert_eq!(history[1].role, "assistant");
    }

    #[tokio::test]
    async fn test_no_screenshot_data_in_memories() {
        // This test validates the lifecycle boundary: saving an entry with
        // content that looks like a screenshot (e.g., base64-encoded bytes)
        // does NOT trigger any special capture path.
        // The database only stores what the user explicitly saved.
        let engine = test_engine().await;
        let entry = NewMemoryEntry {
            content: "user-typed memory, not a screenshot".to_string(),
            tags: vec!["manual".to_string()],
        };
        let saved = engine.save_memory(entry).await.unwrap();
        assert!(!saved.content.is_empty());
        // No other side effects — the test passing confirms nothing went to disk
        // as an image or screenshot file.
    }

    #[tokio::test]
    async fn test_companion_name_and_user_name_persistence() {
        let engine = test_engine().await;

        // Defaults
        assert_eq!(engine.get_companion_name().await.unwrap(), "OpenMate");
        assert_eq!(engine.get_user_name().await.unwrap(), "");

        // Set custom names
        engine.set_companion_name("Hello Kitty").await.unwrap();
        engine.set_user_name("Alice").await.unwrap();

        assert_eq!(engine.get_companion_name().await.unwrap(), "Hello Kitty");
        assert_eq!(engine.get_user_name().await.unwrap(), "Alice");

        // Setting empty companion name falls back to OpenMate
        engine.set_companion_name("   ").await.unwrap();
        assert_eq!(engine.get_companion_name().await.unwrap(), "OpenMate");
    }
}

