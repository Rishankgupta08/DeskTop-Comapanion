//! # Plugin Loader
//!
//! Orchestrates the full cryptographic verification pipeline and loading of OpenMate native plugins. [DR-039, DR-040]

use super::crypto::{CryptoError, PluginVerifier};
use super::manifest::{ManifestError, PluginManifest};
use super::trust::{TrustLevel, TrustRegistry};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

const CURRENT_OPENMATE_VERSION: &str = "0.1.0";

#[derive(Debug, Error)]
pub enum PluginLoadError {
    #[error("Invalid plugin ID '{0}': must be 2-32 lowercase alphanumeric characters with hyphens")]
    InvalidId(String),

    #[error("Manifest plugin.toml not found in plugin directory")]
    ManifestNotFound,

    #[error("Manifest parse error: {0}")]
    ManifestParseError(String),

    #[error("Signature file plugin.sig not found in plugin directory")]
    SignatureNotFound,

    #[error("Cryptographic verification failed: {0}")]
    CryptoError(#[from] CryptoError),

    #[error("Untrusted author key '{pubkey}' (Developer Mode required to run unsigned/untrusted plugins)")]
    UntrustedAuthor { pubkey: String },

    #[error("Revoked author key '{pubkey}' (this key has been revoked and cannot be run)")]
    RevokedAuthor { pubkey: String },

    #[error("Binary executable not found for target platform '{platform}' at '{path}'")]
    BinaryNotFound { platform: String, path: String },

    #[error("Unsupported platform: current operating system/architecture is not supported by this plugin")]
    UnsupportedPlatform,

    #[error("Incompatible OpenMate version: requires '{required}', current is '{current}'")]
    IncompatibleVersion { required: String, current: String },

    #[error("I/O error reading plugin files: {0}")]
    IoError(#[from] std::io::Error),
}

#[derive(Debug, Clone)]
pub struct LoadedPlugin {
    pub manifest: PluginManifest,
    pub trust_level: TrustLevel,
    pub binary_path: PathBuf,
    pub plugin_dir: PathBuf,
}

pub struct PluginLoader {
    pub plugins_dir: PathBuf,
    pub trust: Arc<RwLock<TrustRegistry>>,
}

impl PluginLoader {
    /// Create a new PluginLoader with a base plugins directory and shared trust registry.
    pub fn new(plugins_dir: PathBuf, trust: Arc<RwLock<TrustRegistry>>) -> Self {
        Self { plugins_dir, trust }
    }

    /// Full verification and loading pipeline for a single plugin.
    pub async fn load_and_verify(
        &self,
        plugin_id: &str,
        developer_mode: bool,
    ) -> Result<LoadedPlugin, PluginLoadError> {
        // 1. Validate plugin_id format (prevents directory traversal)
        PluginManifest::validate_id(plugin_id)
            .map_err(|_| PluginLoadError::InvalidId(plugin_id.to_string()))?;

        let plugin_dir = self.plugins_dir.join(plugin_id);
        let manifest_path = plugin_dir.join("plugin.toml");
        let sig_path = plugin_dir.join("plugin.sig");

        // 2. Read and parse plugin.toml
        if !manifest_path.is_file() {
            return Err(PluginLoadError::ManifestNotFound);
        }

        let manifest_bytes = tokio::fs::read(&manifest_path).await?;
        let manifest_str = std::str::from_utf8(&manifest_bytes)
            .map_err(|e| PluginLoadError::ManifestParseError(e.to_string()))?;

        let manifest = PluginManifest::parse_and_validate(manifest_str)
            .map_err(|e| PluginLoadError::ManifestParseError(e.to_string()))?;

        if manifest.plugin.id != plugin_id {
            return Err(PluginLoadError::InvalidId(format!(
                "Manifest ID '{}' does not match directory '{}'",
                manifest.plugin.id, plugin_id
            )));
        }

        // 3. Resolve target platform binary
        let rel_binary_str = manifest
            .current_platform_binary()
            .ok_or(PluginLoadError::UnsupportedPlatform)?;

        let binary_path = plugin_dir.join(rel_binary_str);
        if !binary_path.is_file() {
            return Err(PluginLoadError::BinaryNotFound {
                platform: std::env::consts::OS.to_string(),
                path: binary_path.display().to_string(),
            });
        }

        let binary_bytes = tokio::fs::read(&binary_path).await?;

        // 4. Read signature and verify cryptographic integrity
        let sig_exists = sig_path.is_file();
        let sig_valid = if sig_exists {
            let sig_bytes = tokio::fs::read(&sig_path).await?;
            let payload_hash =
                PluginVerifier::compute_payload_hash(&manifest_bytes, &binary_bytes);

            match PluginVerifier::verify_signature(
                &manifest.plugin.author_pubkey,
                &sig_bytes,
                &payload_hash,
            ) {
                Ok(()) => true,
                Err(e) => {
                    if !developer_mode {
                        return Err(PluginLoadError::CryptoError(e));
                    }
                    false
                }
            }
        } else {
            if !developer_mode {
                return Err(PluginLoadError::SignatureNotFound);
            }
            false
        };

        // 5. Evaluate Trust Level
        let trust_guard = self.trust.read().await;
        let trust_level = if sig_valid {
            trust_guard.is_trusted(&manifest.plugin.author_pubkey)
        } else {
            TrustLevel::Unknown
        };

        match trust_level {
            TrustLevel::Revoked => {
                return Err(PluginLoadError::RevokedAuthor {
                    pubkey: manifest.plugin.author_pubkey.clone(),
                });
            }
            TrustLevel::Unknown => {
                if !developer_mode {
                    return Err(PluginLoadError::UntrustedAuthor {
                        pubkey: manifest.plugin.author_pubkey.clone(),
                    });
                }
            }
            TrustLevel::Builtin | TrustLevel::Community | TrustLevel::UserApproved => {
                // Verified author
            }
        }

        // 6. Check openmate_version compatibility
        if !Self::check_version_compat(
            &manifest.plugin.openmate_version,
            CURRENT_OPENMATE_VERSION,
        ) {
            return Err(PluginLoadError::IncompatibleVersion {
                required: manifest.plugin.openmate_version.clone(),
                current: CURRENT_OPENMATE_VERSION.to_string(),
            });
        }

        info!(
            "[PluginLoader] Successfully verified and loaded plugin '{}' (trust level: {:?})",
            manifest.plugin.id, trust_level
        );

        Ok(LoadedPlugin {
            manifest,
            trust_level,
            binary_path,
            plugin_dir,
        })
    }

    /// Scan all plugin directories in `plugins_dir` and load them.
    pub async fn scan_all(
        &self,
        developer_mode: bool,
    ) -> Vec<Result<LoadedPlugin, PluginLoadError>> {
        let mut results = Vec::new();

        if !self.plugins_dir.is_dir() {
            return results;
        }

        let mut entries = match tokio::fs::read_dir(&self.plugins_dir).await {
            Ok(e) => e,
            Err(_) => return results,
        };

        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path.is_dir() {
                if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
                    if PluginManifest::validate_id(file_name).is_ok() {
                        let res = self.load_and_verify(file_name, developer_mode).await;
                        results.push(res);
                    }
                }
            }
        }

        results
    }

    /// Basic SemVer compatibility check (e.g. ">=0.1.0").
    fn check_version_compat(req: &str, current: &str) -> bool {
        let trimmed = req.trim();
        if trimmed.is_empty() || trimmed == "*" {
            return true;
        }

        if let Some(min_v) = trimmed.strip_prefix(">=") {
            let min_parts: Vec<u32> = min_v
                .split('.')
                .filter_map(|s| s.parse::<u32>().ok())
                .collect();
            let cur_parts: Vec<u32> = current
                .split('.')
                .filter_map(|s| s.parse::<u32>().ok())
                .collect();

            if min_parts.len() == 3 && cur_parts.len() == 3 {
                return cur_parts >= min_parts;
            }
        }

        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_test_plugin_dir(dir: &Path, with_sig: bool) {
        let manifest_content = r#"
[plugin]
id = "test-plugin"
name = "Test Plugin"
version = "1.0.0"
author = "Unknown Author"
author_pubkey = "ed25519:1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef"
description = "Test Description"
openmate_version = ">=0.1.0"

[capabilities]
required = ["network_access"]

[entrypoint]
macos_arm64 = "bin/test"
macos_x86_64 = "bin/test"
windows_x86_64 = "bin/test"
"#;

        let plugin_dir = dir.join("test-plugin");
        let bin_dir = plugin_dir.join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();

        std::fs::write(plugin_dir.join("plugin.toml"), manifest_content).unwrap();
        std::fs::write(bin_dir.join("test"), b"binary_content").unwrap();

        if with_sig {
            std::fs::write(plugin_dir.join("plugin.sig"), vec![0u8; 64]).unwrap();
        }
    }

    #[tokio::test]
    async fn test_reject_unsigned_plugin_non_dev_mode() {
        let temp_dir = std::env::temp_dir().join(format!("test_loader_{}", uuid::Uuid::new_v4()));
        setup_test_plugin_dir(&temp_dir, false);

        let trust = Arc::new(RwLock::new(TrustRegistry::load_bundled().unwrap()));
        let loader = PluginLoader::new(temp_dir.clone(), trust);

        let result = loader.load_and_verify("test-plugin", false).await;
        assert!(result.is_err(), "Unsigned plugin must be rejected when developer_mode=false");
        
        let _ = std::fs::remove_dir_all(temp_dir);
    }

    #[tokio::test]
    async fn test_allow_unsigned_plugin_dev_mode() {
        let temp_dir = std::env::temp_dir().join(format!("test_loader_{}", uuid::Uuid::new_v4()));
        setup_test_plugin_dir(&temp_dir, false);

        let trust = Arc::new(RwLock::new(TrustRegistry::load_bundled().unwrap()));
        let loader = PluginLoader::new(temp_dir.clone(), trust);

        let result = loader.load_and_verify("test-plugin", true).await;
        assert!(result.is_ok(), "Unsigned plugin should load when developer_mode=true");
        let loaded = result.unwrap();
        assert_eq!(loaded.trust_level, TrustLevel::Unknown);

        let _ = std::fs::remove_dir_all(temp_dir);
    }
}
