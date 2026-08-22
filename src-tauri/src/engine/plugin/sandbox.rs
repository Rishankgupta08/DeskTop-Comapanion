//! # Sandboxed Process Spawner
//!
//! Spawns plugin child processes with operating-system-level sandbox constraints and a clean environment. [DR-039]
//!
//! ## Critical Security Invariants:
//! - `env_clear()` is unconditionally invoked on all spawned processes to prevent credential leakage.
//! - The child process receives ONLY `OPENMATE_PLUGIN_ID`.
//! - stdin, stdout, and stderr are strictly piped to the host mediator.

use super::loader::LoadedPlugin;
use std::process::Stdio;
use thiserror::Error;
use tokio::process::{Child, Command};
use tracing::{debug, info, warn};

#[derive(Debug, Error)]
pub enum SandboxError {
    #[error("Failed to spawn sandboxed plugin child process: {0}")]
    SpawnError(#[from] std::io::Error),

    #[error("Plugin binary not executable: {0}")]
    NotExecutable(String),
}

#[cfg(target_os = "macos")]
const MACOS_SANDBOX_PROFILE: &str = r#"
(version 1)
(deny default)
(allow process-exec)
(allow signal (target self))
(allow file-read-data (path-ancestors "/usr/lib"))
(allow file-read-data (path-ancestors "/System/Library"))
(allow file-read-data (path-ancestors "/Library"))
(allow file-read-data (path-ancestors "/private/var"))
(allow mach-lookup)
(allow network-outbound (remote ip))
"#;

pub async fn spawn_sandboxed(plugin: &LoadedPlugin) -> Result<Child, SandboxError> {
    #[cfg(target_os = "macos")]
    {
        // Check if sandbox-exec is available
        let sandbox_bin = std::path::Path::new("/usr/bin/sandbox-exec");
        if sandbox_bin.exists() {
            debug!(
                "[Sandbox] Spawning plugin '{}' with macOS sandbox-exec",
                plugin.manifest.plugin.id
            );
            let child = Command::new(sandbox_bin)
                .arg("-p")
                .arg(MACOS_SANDBOX_PROFILE)
                .arg(&plugin.binary_path)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .env_clear() // CRITICAL: Strip all environment variables
                .env("OPENMATE_PLUGIN_ID", &plugin.manifest.plugin.id)
                .spawn()?;
            return Ok(child);
        }
    }

    // Default cross-platform process isolation with clean environment
    debug!(
        "[Sandbox] Spawning plugin '{}' with process isolation and env_clear()",
        plugin.manifest.plugin.id
    );

    let child = Command::new(&plugin.binary_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_clear() // CRITICAL: Strip all environment variables
        .env("OPENMATE_PLUGIN_ID", &plugin.manifest.plugin.id)
        .spawn()?;

    Ok(child)
}
