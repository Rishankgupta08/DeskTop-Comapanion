//! # Plugin Manifest Parser & Validator
//!
//! Parses and validates `plugin.toml` manifests with strict schema validation. [DR-040, DR-044]
//!
//! ## Security rules:
//! - `#[serde(deny_unknown_fields)]` rejects unexpected fields to prevent metadata injection.
//! - Plugin ID and tool names must adhere strictly to safe regex slug formats.
//! - Required capabilities must belong to the approved capability whitelist.

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const KNOWN_CAPABILITIES: &[&str] = &[
    "network_access",
    "filesystem_read",
    "filesystem_write",
    "screen_capture",
    "microphone",
    "clipboard",
    "app_launch",
];

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ManifestError {
    #[error("TOML deserialization error: {0}")]
    ParseError(String),

    #[error("Invalid plugin ID '{0}': must be 2-32 lowercase alphanumeric characters with hyphens")]
    InvalidId(String),

    #[error("Invalid SemVer version '{0}'")]
    InvalidVersion(String),

    #[error("Invalid author public key format '{0}': must start with 'ed25519:'")]
    InvalidPublicKey(String),

    #[error("Unknown capability '{0}': must be one of {KNOWN_CAPABILITIES:?}")]
    UnknownCapability(String),

    #[error("Too many tools declared ({0}): maximum allowed is 20 tools per plugin")]
    TooManyTools(usize),

    #[error("Invalid tool name '{0}': must be 2-32 lowercase alphanumeric characters with underscores")]
    InvalidToolName(String),

    #[error("Missing entrypoint for active target platform")]
    MissingEntrypoint,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PluginManifest {
    pub plugin: PluginMeta,
    #[serde(default)]
    pub capabilities: PluginCapabilities,
    pub entrypoint: PluginEntrypoint,
    #[serde(default)]
    pub tools: Vec<PluginTool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PluginMeta {
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub author_pubkey: String, // "ed25519:<hex>"
    pub description: String,
    pub openmate_version: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PluginCapabilities {
    #[serde(default)]
    pub required: Vec<String>,
    #[serde(default)]
    pub optional: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PluginEntrypoint {
    pub macos_arm64: Option<String>,
    pub macos_x86_64: Option<String>,
    pub windows_x86_64: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PluginTool {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

impl PluginManifest {
    /// Parse and strictly validate a `plugin.toml` string.
    pub fn parse_and_validate(toml_str: &str) -> Result<Self, ManifestError> {
        let manifest: PluginManifest =
            toml::from_str(toml_str).map_err(|e| ManifestError::ParseError(e.to_string()))?;

        manifest.validate()?;
        Ok(manifest)
    }

    /// Perform exhaustive validation checks on manifest properties.
    pub fn validate(&self) -> Result<(), ManifestError> {
        // 1. Validate plugin ID
        Self::validate_id(&self.plugin.id)?;

        // 2. Validate SemVer version
        Self::validate_semver(&self.plugin.version)?;

        // 3. Validate author public key prefix
        if !self.plugin.author_pubkey.trim().starts_with("ed25519:") {
            return Err(ManifestError::InvalidPublicKey(self.plugin.author_pubkey.clone()));
        }

        // 4. Validate capabilities against whitelist
        for cap in &self.capabilities.required {
            if !KNOWN_CAPABILITIES.contains(&cap.as_str()) {
                return Err(ManifestError::UnknownCapability(cap.clone()));
            }
        }
        for cap in &self.capabilities.optional {
            if !KNOWN_CAPABILITIES.contains(&cap.as_str()) {
                return Err(ManifestError::UnknownCapability(cap.clone()));
            }
        }

        // 5. Validate tools count and names
        if self.tools.len() > 20 {
            return Err(ManifestError::TooManyTools(self.tools.len()));
        }

        for tool in &self.tools {
            Self::validate_tool_name(&tool.name)?;
        }

        Ok(())
    }

    /// Validate plugin ID slug format (/^[a-z0-9-]{2,32}$/ without leading/trailing hyphens).
    pub fn validate_id(id: &str) -> Result<(), ManifestError> {
        let trimmed = id.trim();
        if trimmed.len() < 2 || trimmed.len() > 32 {
            return Err(ManifestError::InvalidId(id.to_string()));
        }

        if trimmed.starts_with('-') || trimmed.ends_with('-') {
            return Err(ManifestError::InvalidId(id.to_string()));
        }

        for c in trimmed.chars() {
            if !c.is_ascii_lowercase() && !c.is_ascii_digit() && c != '-' {
                return Err(ManifestError::InvalidId(id.to_string()));
            }
        }

        Ok(())
    }

    /// Validate tool name format (/^[a-z0-9_]{2,32}$/).
    pub fn validate_tool_name(name: &str) -> Result<(), ManifestError> {
        let trimmed = name.trim();
        if trimmed.len() < 2 || trimmed.len() > 32 {
            return Err(ManifestError::InvalidToolName(name.to_string()));
        }

        for c in trimmed.chars() {
            if !c.is_ascii_lowercase() && !c.is_ascii_digit() && c != '_' {
                return Err(ManifestError::InvalidToolName(name.to_string()));
            }
        }

        Ok(())
    }

    /// Basic SemVer validation (X.Y.Z[-prerelease]).
    pub fn validate_semver(version: &str) -> Result<(), ManifestError> {
        let trimmed = version.trim();
        let parts: Vec<&str> = trimmed.split('.').collect();
        if parts.len() < 3 {
            return Err(ManifestError::InvalidVersion(version.to_string()));
        }

        // Major, minor must parse as u32
        if parts[0].parse::<u32>().is_err() || parts[1].parse::<u32>().is_err() {
            return Err(ManifestError::InvalidVersion(version.to_string()));
        }

        // Patch may contain prerelease tag
        let patch_part = parts[2].split('-').next().unwrap_or(parts[2]);
        if patch_part.parse::<u32>().is_err() {
            return Err(ManifestError::InvalidVersion(version.to_string()));
        }

        Ok(())
    }

    /// Get relative binary path for current target platform.
    pub fn current_platform_binary(&self) -> Option<&str> {
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        {
            self.entrypoint.macos_arm64.as_deref()
        }

        #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
        {
            self.entrypoint.macos_x86_64.as_deref()
        }

        #[cfg(target_os = "windows")]
        {
            self.entrypoint.windows_x86_64.as_deref()
        }

        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_manifest_rejects_unknown_capability() {
        let toml_str = r#"
[plugin]
id = "test-plugin"
name = "Test"
version = "1.0.0"
author = "Tester"
author_pubkey = "ed25519:3b6f2c7d9e1a4f5b8c0d2e3f4a5b6c7d8e9f0a1b2c3d4e5f6a7b8c9d0e1f2a3b"
description = "Test"
openmate_version = ">=0.1.0"

[capabilities]
required = ["nuclear_launch"]

[entrypoint]
macos_arm64 = "bin/test"
"#;

        let result = PluginManifest::parse_and_validate(toml_str);
        assert_eq!(
            result,
            Err(ManifestError::UnknownCapability("nuclear_launch".to_string()))
        );
    }

    #[test]
    fn test_manifest_rejects_more_than_20_tools() {
        let mut tools_toml = String::new();
        for i in 0..21 {
            tools_toml.push_str(&format!(
                "[[tools]]\nname = \"tool_{}\"\ndescription = \"desc\"\nparameters = {{}}\n",
                i
            ));
        }

        let toml_str = format!(
            r#"
[plugin]
id = "many-tools"
name = "Many Tools"
version = "1.0.0"
author = "Tester"
author_pubkey = "ed25519:3b6f2c7d9e1a4f5b8c0d2e3f4a5b6c7d8e9f0a1b2c3d4e5f6a7b8c9d0e1f2a3b"
description = "Test"
openmate_version = ">=0.1.0"

[capabilities]
required = ["network_access"]

[entrypoint]
macos_arm64 = "bin/test"

{}"#,
            tools_toml
        );

        let result = PluginManifest::parse_and_validate(&toml_str);
        assert_eq!(result, Err(ManifestError::TooManyTools(21)));
    }

    #[test]
    fn test_manifest_rejects_invalid_id_slug() {
        assert!(PluginManifest::validate_id("-invalid-start").is_err());
        assert!(PluginManifest::validate_id("invalid-end-").is_err());
        assert!(PluginManifest::validate_id("UPPERCASE").is_err());
        assert!(PluginManifest::validate_id("a").is_err()); // < 2 chars
        assert!(PluginManifest::validate_id("valid-plugin-id").is_ok());
    }
}
