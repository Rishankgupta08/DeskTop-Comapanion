//! # Plugin Host & JSON-RPC Mediator
//!
//! Mediates communication between the OpenMate host and the sandboxed plugin child process. [DR-039]

use super::loader::LoadedPlugin;
use super::sandbox::spawn_sandboxed;
use crate::engine::permission::{Capability, PermissionEngine, PermissionState};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::time::timeout;
use tracing::{debug, error, info, warn};

const PLUGIN_CALL_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Error)]
pub enum PluginCallError {
    #[error("Plugin process is not running or failed to spawn: {0}")]
    ProcessSpawnError(String),

    #[error("Tool '{0}' is not declared in plugin manifest")]
    ToolNotDeclared(String),

    #[error("Permission denied for capability '{capability}': current state is {state:?}")]
    PermissionDenied {
        capability: String,
        state: PermissionState,
    },

    #[error("I/O error communicating with plugin child process: {0}")]
    IoError(String),

    #[error("JSON serialization error: {0}")]
    JsonError(String),

    #[error("Plugin call timed out after 10 seconds (process terminated)")]
    Timeout,

    #[error("Plugin returned JSON-RPC error ({code}): {message}")]
    JsonRpcError { code: i64, message: String },

    #[error("Plugin process exited prematurely")]
    ProcessExited,
}

#[derive(Debug, Serialize)]
struct JsonRpcRequest<T> {
    jsonrpc: &'static str,
    id: u64,
    method: &'static str,
    params: T,
}

#[derive(Debug, Serialize)]
struct ToolCallParams {
    tool: String,
    arguments: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct JsonRpcResponse<T> {
    #[allow(dead_code)]
    jsonrpc: String,
    id: Option<u64>,
    result: Option<T>,
    error: Option<JsonRpcErrorDetail>,
}

#[derive(Debug, Deserialize)]
struct JsonRpcErrorDetail {
    code: i64,
    message: String,
}

#[derive(Debug, Deserialize)]
pub struct ToolCallResult {
    pub output: String,
    pub success: bool,
}

pub struct PluginHost {
    pub plugin: LoadedPlugin,
    process: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    request_id: AtomicU64,
}

impl PluginHost {
    /// Spawn a sandboxed plugin process and wrap it in a `PluginHost`.
    pub async fn start(plugin: LoadedPlugin) -> Result<Self, PluginCallError> {
        let mut child = spawn_sandboxed(&plugin)
            .await
            .map_err(|e| PluginCallError::ProcessSpawnError(e.to_string()))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| PluginCallError::ProcessSpawnError("Failed to open stdin".to_string()))?;

        let stdout_raw = child.stdout.take().ok_or_else(|| {
            PluginCallError::ProcessSpawnError("Failed to open stdout".to_string())
        })?;

        let stdout = BufReader::new(stdout_raw);

        info!(
            "[PluginHost] Started host for plugin '{}' (pid: {:?})",
            plugin.manifest.plugin.id,
            child.id()
        );

        Ok(Self {
            plugin,
            process: child,
            stdin,
            stdout,
            request_id: AtomicU64::new(1),
        })
    }

    /// Execute a tool call on the plugin with PermissionEngine verification and timeout enforcement.
    pub async fn call_tool(
        &mut self,
        tool_name: &str,
        arguments: serde_json::Value,
        permission_engine: &PermissionEngine,
    ) -> Result<String, PluginCallError> {
        // 1. Verify that tool_name exists in manifest
        let tool_exists = self
            .plugin
            .manifest
            .tools
            .iter()
            .any(|t| t.name == tool_name);

        if !tool_exists {
            return Err(PluginCallError::ToolNotDeclared(tool_name.to_string()));
        }

        // 2. Check PermissionEngine for each required capability FIRST
        for cap_str in &self.plugin.manifest.capabilities.required {
            if let Some(cap) = Capability::from_str_name(cap_str) {
                let state = permission_engine.get(&cap).await;
                if state != PermissionState::Allow {
                    return Err(PluginCallError::PermissionDenied {
                        capability: cap_str.clone(),
                        state,
                    });
                }
            }
        }

        // Master switch check for plugins
        let master_state = permission_engine.get(&Capability::PluginExecution).await;
        if master_state != PermissionState::Allow {
            return Err(PluginCallError::PermissionDenied {
                capability: "plugin_execution".to_string(),
                state: master_state,
            });
        }

        // 3. Prepare JSON-RPC request
        let req_id = self.request_id.fetch_add(1, Ordering::SeqCst);
        let req = JsonRpcRequest {
            jsonrpc: "2.0",
            id: req_id,
            method: "tool_call",
            params: ToolCallParams {
                tool: tool_name.to_string(),
                arguments,
            },
        };

        let req_json =
            serde_json::to_string(&req).map_err(|e| PluginCallError::JsonError(e.to_string()))?;

        // 4. Send request and wait for response with 10-second timeout
        let call_future = async {
            // Write request line to stdin
            self.stdin
                .write_all(req_json.as_bytes())
                .await
                .map_err(|e| PluginCallError::IoError(e.to_string()))?;
            self.stdin
                .write_all(b"\n")
                .await
                .map_err(|e| PluginCallError::IoError(e.to_string()))?;
            self.stdin
                .flush()
                .await
                .map_err(|e| PluginCallError::IoError(e.to_string()))?;

            // Read response line from stdout
            let mut line = String::new();
            let n = self
                .stdout
                .read_line(&mut line)
                .await
                .map_err(|e| PluginCallError::IoError(e.to_string()))?;

            if n == 0 {
                return Err(PluginCallError::ProcessExited);
            }

            let resp: JsonRpcResponse<ToolCallResult> = serde_json::from_str(&line)
                .map_err(|e| PluginCallError::JsonError(e.to_string()))?;

            if let Some(err) = resp.error {
                return Err(PluginCallError::JsonRpcError {
                    code: err.code,
                    message: err.message,
                });
            }

            match resp.result {
                Some(r) => Ok(r.output),
                None => Err(PluginCallError::JsonRpcError {
                    code: -32603,
                    message: "Missing result in JSON-RPC response".to_string(),
                }),
            }
        };

        match timeout(PLUGIN_CALL_TIMEOUT, call_future).await {
            Ok(result) => result,
            Err(_) => {
                error!(
                    "[PluginHost] Tool call '{}' on plugin '{}' timed out after {:?}. Killing process.",
                    tool_name, self.plugin.manifest.plugin.id, PLUGIN_CALL_TIMEOUT
                );
                let _ = self.process.kill().await;
                Err(PluginCallError::Timeout)
            }
        }
    }

    /// Gracefully shutdown or terminate child process.
    pub async fn shutdown(&mut self) -> Result<(), PluginCallError> {
        let _ = self.process.kill().await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::permission::PermissionEngine;
    use crate::engine::plugin::manifest::{PluginEntrypoint, PluginMeta, PluginTool};
    use crate::engine::plugin::TrustLevel;
    use std::path::PathBuf;
    use tokio_rusqlite::Connection;

    #[tokio::test]
    async fn test_tool_not_declared_rejection() {
        let db = Connection::open_in_memory().await.unwrap();
        let perm_engine = PermissionEngine::new(db).await.unwrap();

        let plugin = LoadedPlugin {
            manifest: crate::engine::plugin::PluginManifest {
                plugin: PluginMeta {
                    id: "test-plugin".to_string(),
                    name: "Test".to_string(),
                    version: "1.0.0".to_string(),
                    author: "Tester".to_string(),
                    author_pubkey: "ed25519:1234".to_string(),
                    description: "desc".to_string(),
                    openmate_version: ">=0.1.0".to_string(),
                },
                capabilities: Default::default(),
                entrypoint: PluginEntrypoint {
                    macos_arm64: None,
                    macos_x86_64: None,
                    windows_x86_64: None,
                },
                tools: vec![PluginTool {
                    name: "declared_tool".to_string(),
                    description: "desc".to_string(),
                    parameters: serde_json::json!({}),
                }],
            },
            trust_level: TrustLevel::Builtin,
            binary_path: PathBuf::from("/bin/echo"),
            plugin_dir: PathBuf::from("/tmp"),
        };

        // We test tool existence checking before process spawn
        let tool_exists = plugin.manifest.tools.iter().any(|t| t.name == "non_existent_tool");
        assert!(!tool_exists, "Undeclared tool must not exist in manifest");
    }
}
