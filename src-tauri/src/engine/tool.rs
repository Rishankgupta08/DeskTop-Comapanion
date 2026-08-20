// Tool schema CONFIRMED [DR-018]
// Assistant: open_application, read_file, write_file
// Coder: read_file, list_directory, search_in_file
// All tools require PermissionToken from PermissionEngine before execution

//! # Tool Engine
//!
//! Executes policy-gated built-in actions for Coder and Assistant modes.
//! [DR-018, DR-029, DR-033, ADR-011]
//!
//! ## Security rules (non-negotiable):
//! - Every tool checks `PermissionEngine` before any OS call. [PINJ-004]
//! - Path traversal sequences (`..`) and non-absolute paths are strictly rejected.
//! - Shell metacharacters in application names are rejected.
//! - `write_file` refuses to overwrite existing files without explicit user action.
//! - File operations enforce extension allowlists and file size ceilings.

use crate::{
    engine::permission::{Capability, PermissionEngine, PermissionToken},
    error::OpenMateError,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{debug, error, info};

const SUPPORTED_EXTENSIONS: &[&str] = &[
    "txt", "md", "rs", "py", "js", "ts", "json", "toml", "yaml", "yml", "html", "css",
];

const MAX_READ_BYTES: usize = 100 * 1024; // 100 KB
const MAX_WRITE_BYTES: usize = 50 * 1024; // 50 KB
const MAX_DIR_ENTRIES: usize = 200;
const MAX_SEARCH_MATCHES: usize = 50;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "tool", rename_all = "snake_case")]
pub enum Tool {
    // Assistant Mode tools
    OpenApplication { name: String },
    ReadFile { path: PathBuf },
    WriteFile {
        path: PathBuf,
        content: String,
        #[serde(default)]
        overwrite: bool,
        #[serde(default)]
        create_copy: bool,
    },
    // Coder Mode tools
    ListDirectory { path: PathBuf },
    SearchInFile { path: PathBuf, query: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub tool: String,
    pub success: bool,
    pub output: String, // human-readable result
    pub error: Option<String>,
}

pub struct ToolEngine {
    #[allow(dead_code)]
    permission_engine: Arc<PermissionEngine>,
}

impl ToolEngine {
    pub fn new(permission_engine: Arc<PermissionEngine>) -> Self {
        Self { permission_engine }
    }

    /// Execute a tool call with permission check and compile-time token acquisition.
    pub async fn execute(
        &self,
        tool: Tool,
        permission_engine: &PermissionEngine,
    ) -> Result<ToolResult, OpenMateError> {
        match tool {
            Tool::OpenApplication { name } => {
                let token = permission_engine.check(Capability::AppLaunch).await?;
                Self::open_application(&token, name).await
            }
            Tool::ReadFile { path } => {
                let token = permission_engine.check(Capability::FilesystemRead).await?;
                Self::read_file(&token, path).await
            }
            Tool::WriteFile {
                path,
                content,
                overwrite,
                create_copy,
            } => {
                let token = permission_engine.check(Capability::FilesystemWrite).await?;
                Self::write_file(&token, path, content, overwrite, create_copy).await
            }
            Tool::ListDirectory { path } => {
                let token = permission_engine.check(Capability::FilesystemRead).await?;
                Self::list_directory(&token, path).await
            }
            Tool::SearchInFile { path, query } => {
                let token = permission_engine.check(Capability::FilesystemRead).await?;
                Self::search_in_file(&token, path, query).await
            }
        }
    }

    // ── 3.1 open_application ───────────────────────────────────────────────────

    pub async fn open_application(
        _token: &PermissionToken,
        name: String,
    ) -> Result<ToolResult, OpenMateError> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err(OpenMateError::InvalidToolArgument(
                "Application name must not be empty".to_string(),
            ));
        }

        // Sanitize: reject shell metacharacters
        let forbidden = [
            ';', '&', '|', '>', '<', '`', '$', '(', ')', '{', '}', '[', ']', '\\', '\'', '"',
        ];
        if trimmed.chars().any(|c| forbidden.contains(&c)) {
            return Err(OpenMateError::InvalidToolArgument(
                "Application name contains invalid shell characters".to_string(),
            ));
        }

        info!("Tool: opening application '{}'", trimmed);

        #[cfg(target_os = "macos")]
        let mut cmd = {
            let mut c = std::process::Command::new("open");
            c.arg("-a").arg(trimmed);
            c
        };

        #[cfg(target_os = "windows")]
        let mut cmd = {
            let mut c = std::process::Command::new("cmd");
            c.args(["/C", "start", "", trimmed]);
            c
        };

        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let mut cmd = {
            let mut c = std::process::Command::new("gtk-launch");
            c.arg(trimmed);
            c
        };

        match cmd.spawn() {
            Ok(_) => Ok(ToolResult {
                tool: "open_application".to_string(),
                success: true,
                output: format!("Opened application '{}'", trimmed),
                error: None,
            }),
            Err(e) => {
                error!("Failed to launch application '{}': {}", trimmed, e);
                Ok(ToolResult {
                    tool: "open_application".to_string(),
                    success: false,
                    output: String::new(),
                    error: Some(format!("Could not launch application '{}': {}", trimmed, e)),
                })
            }
        }
    }

    // ── 3.2 read_file ─────────────────────────────────────────────────────────

    pub async fn read_file(
        _token: &PermissionToken,
        path: PathBuf,
    ) -> Result<ToolResult, OpenMateError> {
        Self::validate_path(&path)?;
        Self::validate_extension(&path)?;

        let metadata = tokio::fs::metadata(&path)
            .await
            .map_err(|e| OpenMateError::ToolError(format!("Cannot access file {:?}: {}", path, e)))?;

        let file_size = metadata.len() as usize;
        let bytes = tokio::fs::read(&path)
            .await
            .map_err(|e| OpenMateError::ToolError(format!("Failed to read {:?}: {}", path, e)))?;

        let output = if file_size > MAX_READ_BYTES {
            let truncated = String::from_utf8_lossy(&bytes[..MAX_READ_BYTES]);
            format!(
                "[File truncated at 100KB. Full file is {} bytes.]\n{}",
                file_size, truncated
            )
        } else {
            String::from_utf8_lossy(&bytes).to_string()
        };

        debug!("Read {} bytes from {:?}", bytes.len(), path);

        Ok(ToolResult {
            tool: "read_file".to_string(),
            success: true,
            output,
            error: None,
        })
    }

    // ── 3.3 write_file ────────────────────────────────────────────────────────

    pub async fn write_file(
        _token: &PermissionToken,
        path: PathBuf,
        content: String,
        overwrite: bool,
        create_copy: bool,
    ) -> Result<ToolResult, OpenMateError> {
        Self::validate_path(&path)?;
        Self::validate_extension(&path)?;

        let target_path = if create_copy {
            let copy_path = generate_non_conflicting_copy_path(&path);
            Self::validate_path(&copy_path)?;
            copy_path
        } else {
            path
        };

        // Do not silently overwrite existing files unless explicitly confirmed
        if target_path.exists() && !overwrite {
            let file_name = target_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("File");
            return Err(OpenMateError::FileAlreadyExists(format!(
                "`{}` already exists at `{}`.",
                file_name,
                target_path.to_string_lossy()
            )));
        }

        let byte_len = content.as_bytes().len();
        if byte_len > MAX_WRITE_BYTES {
            return Err(OpenMateError::InvalidToolArgument(format!(
                "Write content ({} bytes) exceeds maximum allowable 50KB limit",
                byte_len
            )));
        }

        // Ensure parent directories exist
        if let Some(parent) = target_path.parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }

        tokio::fs::write(&target_path, &content)
            .await
            .map_err(|e| OpenMateError::ToolError(format!("Failed to write to {:?}: {}", target_path, e)))?;

        info!("Tool: wrote {} bytes to {:?}", byte_len, target_path);

        let output_msg = if create_copy {
            format!("Successfully created copy and wrote {} bytes to `{}`", byte_len, target_path.to_string_lossy())
        } else if overwrite {
            format!("Successfully overwrote and wrote {} bytes to `{}`", byte_len, target_path.to_string_lossy())
        } else {
            format!("Successfully wrote {} bytes to `{}`", byte_len, target_path.to_string_lossy())
        };

        Ok(ToolResult {
            tool: "write_file".to_string(),
            success: true,
            output: output_msg,
            error: None,
        })
    }

    // ── 3.4 list_directory ────────────────────────────────────────────────────

    pub async fn list_directory(
        _token: &PermissionToken,
        path: PathBuf,
    ) -> Result<ToolResult, OpenMateError> {
        Self::validate_path(&path)?;

        if !path.is_dir() {
            return Err(OpenMateError::InvalidToolArgument(format!(
                "Path {:?} is not a directory",
                path
            )));
        }

        let mut read_dir = tokio::fs::read_dir(&path)
            .await
            .map_err(|e| OpenMateError::ToolError(format!("Failed to list directory {:?}: {}", path, e)))?;

        let mut entries = Vec::new();
        let mut count = 0;

        while let Ok(Some(entry)) = read_dir.next_entry().await {
            count += 1;
            if entries.len() < MAX_DIR_ENTRIES {
                let file_name = entry.file_name().to_string_lossy().to_string();
                let is_dir = entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false);
                if is_dir {
                    entries.push(format!("{}/", file_name));
                } else {
                    entries.push(file_name);
                }
            }
        }

        entries.sort();

        let mut output = entries.join("\n");
        if count > MAX_DIR_ENTRIES {
            output.push_str(&format!(
                "\n[Showing first {} of {} directory entries]",
                MAX_DIR_ENTRIES, count
            ));
        }

        Ok(ToolResult {
            tool: "list_directory".to_string(),
            success: true,
            output,
            error: None,
        })
    }

    // ── 3.5 search_in_file ────────────────────────────────────────────────────

    pub async fn search_in_file(
        _token: &PermissionToken,
        path: PathBuf,
        query: String,
    ) -> Result<ToolResult, OpenMateError> {
        Self::validate_path(&path)?;
        Self::validate_extension(&path)?;

        let trimmed_query = query.trim();
        if trimmed_query.is_empty() {
            return Err(OpenMateError::InvalidToolArgument(
                "Search query must not be empty".to_string(),
            ));
        }

        let content = tokio::fs::read_to_string(&path)
            .await
            .map_err(|e| OpenMateError::ToolError(format!("Failed to read {:?}: {}", path, e)))?;

        let lower_query = trimmed_query.to_lowercase();
        let mut matches = Vec::new();
        let mut total_matches = 0;

        for (idx, line) in content.lines().enumerate() {
            if line.to_lowercase().contains(&lower_query) {
                total_matches += 1;
                if matches.len() < MAX_SEARCH_MATCHES {
                    matches.push(format!("Line {}: {}", idx + 1, line));
                }
            }
        }

        let mut output = if matches.is_empty() {
            format!("No matches found for '{}' in {:?}", trimmed_query, path)
        } else {
            matches.join("\n")
        };

        if total_matches > MAX_SEARCH_MATCHES {
            output.push_str(&format!(
                "\n[Showing first {} of {} matches]",
                MAX_SEARCH_MATCHES, total_matches
            ));
        }

        Ok(ToolResult {
            tool: "search_in_file".to_string(),
            success: true,
            output,
            error: None,
        })
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn validate_path(path: &Path) -> Result<(), OpenMateError> {
        validate_path_for_os(&path.to_string_lossy(), cfg!(windows))
    }

    fn validate_extension(path: &Path) -> Result<(), OpenMateError> {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        if !SUPPORTED_EXTENSIONS.contains(&ext.as_str()) {
            return Err(OpenMateError::UnsupportedFileType(format!(
                "File extension '.{}' is not supported. Supported extensions: {:?}",
                ext, SUPPORTED_EXTENSIONS
            )));
        }

        Ok(())
    }
}

/// Check if a path string conforms to Windows absolute path syntax (e.g. "C:\...", "D:/...", "\\server\share\...").
pub fn is_windows_absolute_path_str(p: &str) -> bool {
    let bytes = p.as_bytes();
    // Drive letter absolute: "C:\" or "C:/"
    if bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
    {
        return true;
    }
    // UNC path: "\\server\share" or "\\server/share"
    if bytes.len() >= 4
        && bytes[0] == b'\\'
        && bytes[1] == b'\\'
        && bytes[2] != b'\\'
        && bytes[2] != b'/'
    {
        return true;
    }
    false
}

/// Check if a path string conforms to Unix/macOS absolute path syntax (e.g. "/Users/...").
pub fn is_unix_absolute_path_str(p: &str) -> bool {
    p.starts_with('/')
}

/// Validate a path string against platform-specific absolute path rules and security constraints.
pub fn validate_path_for_os(path_str: &str, is_windows_os: bool) -> Result<(), OpenMateError> {
    let trimmed = path_str.trim();
    if trimmed.is_empty() {
        return Err(OpenMateError::InvalidToolArgument(
            "Path must not be empty".to_string(),
        ));
    }

    // Must not contain directory traversal sequences
    if trimmed.contains("..") {
        return Err(OpenMateError::InvalidToolArgument(
            "Path traversal sequences ('..') are strictly prohibited.".to_string(),
        ));
    }

    if is_windows_os {
        if is_unix_absolute_path_str(trimmed) {
            return Err(OpenMateError::InvalidToolArgument(
                "That path belongs to macOS/Linux, but OpenMate is currently running on Windows. Please provide a Windows path (e.g. C:\\Users\\...).".to_string(),
            ));
        }
        if !is_windows_absolute_path_str(trimmed) {
            return Err(OpenMateError::InvalidToolArgument(format!(
                "Path \"{}\" is relative. Only absolute paths are permitted.",
                trimmed
            )));
        }
    } else {
        if is_windows_absolute_path_str(trimmed) {
            return Err(OpenMateError::InvalidToolArgument(
                "That path belongs to Windows, but OpenMate is currently running on macOS. Please provide a macOS path.".to_string(),
            ));
        }
        if !is_unix_absolute_path_str(trimmed) {
            return Err(OpenMateError::InvalidToolArgument(format!(
                "Path \"{}\" is relative. Only absolute paths are permitted.",
                trimmed
            )));
        }
    }

    Ok(())
}

/// Generate a unique, non-conflicting copy path (e.g. "file (1).txt", "file (2).txt").
pub fn generate_non_conflicting_copy_path(path: &Path) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new(""));
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
    let ext = path.extension().and_then(|e| e.to_str());

    let mut counter = 1;
    loop {
        let new_file_name = match ext {
            Some(e) if !e.is_empty() => format!("{} ({}).{}", stem, counter, e),
            _ => format!("{} ({})", stem, counter),
        };
        let candidate = parent.join(new_file_name);
        if !candidate.exists() {
            return candidate;
        }
        counter += 1;
    }
}

// ── Unit Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::permission::PermissionState;
    use tokio_rusqlite::Connection;

    async fn setup_permission_engine(
        read: PermissionState,
        write: PermissionState,
        launch: PermissionState,
    ) -> Arc<PermissionEngine> {
        let db = Connection::open_in_memory().await.unwrap();
        crate::engine::memory::run_migrations(&db).await.unwrap();

        let engine = Arc::new(PermissionEngine::new(db).await.unwrap());
        engine.set_permission(Capability::FilesystemRead, read).await.unwrap();
        engine.set_permission(Capability::FilesystemWrite, write).await.unwrap();
        engine.set_permission(Capability::AppLaunch, launch).await.unwrap();
        engine
    }

    #[tokio::test]
    async fn test_open_application_rejects_shell_metacharacters() {
        let pe = setup_permission_engine(
            PermissionState::Allow,
            PermissionState::Allow,
            PermissionState::Allow,
        )
        .await;
        let te = ToolEngine::new(Arc::clone(&pe));

        let res = te
            .execute(
                Tool::OpenApplication {
                    name: "Safari; rm -rf /".to_string(),
                },
                &pe,
            )
            .await;

        assert!(
            matches!(res, Err(OpenMateError::InvalidToolArgument(_))),
            "Expected InvalidToolArgument for shell metacharacters, got: {:?}",
            res
        );
    }

    #[tokio::test]
    async fn test_read_file_rejects_path_traversal() {
        let pe = setup_permission_engine(
            PermissionState::Allow,
            PermissionState::Allow,
            PermissionState::Allow,
        )
        .await;
        let te = ToolEngine::new(Arc::clone(&pe));

        let res = te
            .execute(
                Tool::ReadFile {
                    path: PathBuf::from("/Users/mac/../../../etc/passwd.txt"),
                },
                &pe,
            )
            .await;

        assert!(
            matches!(res, Err(OpenMateError::InvalidToolArgument(_))),
            "Expected InvalidToolArgument for path traversal, got: {:?}",
            res
        );
    }

    #[tokio::test]
    async fn test_read_file_rejects_unsupported_extension() {
        let pe = setup_permission_engine(
            PermissionState::Allow,
            PermissionState::Allow,
            PermissionState::Allow,
        )
        .await;
        let te = ToolEngine::new(Arc::clone(&pe));

        let res = te
            .execute(
                Tool::ReadFile {
                    path: PathBuf::from("/Users/mac/file.exe"),
                },
                &pe,
            )
            .await;

        assert!(
            matches!(res, Err(OpenMateError::UnsupportedFileType(_))),
            "Expected UnsupportedFileType for .exe, got: {:?}",
            res
        );
    }

    #[tokio::test]
    async fn test_write_file_does_not_overwrite_existing_file() {
        let pe = setup_permission_engine(
            PermissionState::Allow,
            PermissionState::Allow,
            PermissionState::Allow,
        )
        .await;
        let te = ToolEngine::new(Arc::clone(&pe));

        let temp_dir = std::env::temp_dir();
        let temp_file = temp_dir.join(format!("test_openmate_{}.txt", uuid::Uuid::new_v4()));
        std::fs::write(&temp_file, "original content").unwrap();

        let res = te
            .execute(
                Tool::WriteFile {
                    path: temp_file.clone(),
                    content: "new content".to_string(),
                    overwrite: false,
                    create_copy: false,
                },
                &pe,
            )
            .await;

        let _ = std::fs::remove_file(&temp_file);

        assert!(
            matches!(res, Err(OpenMateError::FileAlreadyExists(_))),
            "Expected FileAlreadyExists, got: {:?}",
            res
        );
    }

    #[tokio::test]
    async fn test_all_tools_check_permission_before_executing() {
        let pe = setup_permission_engine(
            PermissionState::Off,
            PermissionState::Off,
            PermissionState::Off,
        )
        .await;
        let te = ToolEngine::new(Arc::clone(&pe));

        let res = te
            .execute(
                Tool::ReadFile {
                    path: PathBuf::from("/Users/mac/file.txt"),
                },
                &pe,
            )
            .await;

        assert!(
            matches!(res, Err(OpenMateError::PermissionDenied(_))),
            "Expected PermissionDenied when FilesystemRead is Off, got: {:?}",
            res
        );
    }

    #[tokio::test]
    async fn test_open_application_denied_when_app_launch_off() {
        let pe = setup_permission_engine(
            PermissionState::Off,
            PermissionState::Off,
            PermissionState::Off, // AppLaunch is Off
        )
        .await;
        let te = ToolEngine::new(Arc::clone(&pe));

        let res = te
            .execute(
                Tool::OpenApplication {
                    name: "Calculator".to_string(),
                },
                &pe,
            )
            .await;

        assert!(
            matches!(res, Err(OpenMateError::PermissionDenied(_))),
            "Expected PermissionDenied when AppLaunch is Off, got: {:?}",
            res
        );

        // Verify the formatted error message
        if let Err(e) = res {
            assert!(
                e.user_message().contains("Launch Applications is disabled"),
                "User message should specify the capability name, got: {}",
                e.user_message()
            );
        }
    }

    #[tokio::test]
    async fn test_open_application_allowed_when_app_launch_allow() {
        let pe = setup_permission_engine(
            PermissionState::Off,
            PermissionState::Off,
            PermissionState::Allow, // AppLaunch is Allow
        )
        .await;
        let te = ToolEngine::new(Arc::clone(&pe));

        // Use standard macOS app that exists or spawn executes
        let res = te
            .execute(
                Tool::OpenApplication {
                    name: "Calculator".to_string(),
                },
                &pe,
            )
            .await;

        assert!(
            res.is_ok(),
            "Expected Ok result when AppLaunch is Allow, got: {:?}",
            res
        );
        let tool_res = res.unwrap();
        assert_eq!(tool_res.tool, "open_application");
        assert!(tool_res.success);
        assert!(tool_res.output.contains("Opened application 'Calculator'"));
    }

    #[test]
    fn test_path_validation_macos_semantics() {
        // Valid macOS absolute paths
        assert!(validate_path_for_os("/Users/test/Documents/file.txt", false).is_ok());
        assert!(validate_path_for_os("/Users/Rishank/Documents/I_am_Rishank.txt", false).is_ok());
        assert!(validate_path_for_os("/tmp/test.txt", false).is_ok());

        // Windows absolute paths rejected with clean OS mismatch message on macOS
        let win_path = validate_path_for_os(r"C:\Users\Test\Documents\file.txt", false);
        assert!(win_path.is_err());
        assert!(win_path.unwrap_err().user_message().contains("belongs to Windows, but OpenMate is currently running on macOS"));

        let win_d = validate_path_for_os(r"D:\test.txt", false);
        assert!(win_d.is_err());
        assert!(win_d.unwrap_err().user_message().contains("belongs to Windows, but OpenMate is currently running on macOS"));

        let unc = validate_path_for_os(r"\\server\share\file.txt", false);
        assert!(unc.is_err());
        assert!(unc.unwrap_err().user_message().contains("belongs to Windows, but OpenMate is currently running on macOS"));

        // Relative paths rejected as relative
        let rel1 = validate_path_for_os("Documents/file.txt", false);
        assert!(rel1.is_err());
        assert!(rel1.unwrap_err().user_message().contains("is relative"));

        let rel2 = validate_path_for_os("./file.txt", false);
        assert!(rel2.is_err());
        assert!(rel2.unwrap_err().user_message().contains("is relative"));

        // Path traversal rejected
        let trav = validate_path_for_os("/Users/test/../etc/passwd", false);
        assert!(trav.is_err());
        assert!(trav.unwrap_err().user_message().contains("Path traversal sequences"));
    }

    #[test]
    fn test_path_validation_windows_semantics() {
        // Valid Windows absolute paths
        assert!(validate_path_for_os(r"C:\Users\Test\Documents\file.txt", true).is_ok());
        assert!(validate_path_for_os(r"D:\test.txt", true).is_ok());
        assert!(validate_path_for_os(r"\\server\share\file.txt", true).is_ok());

        // Unix/macOS absolute paths rejected with clean OS mismatch message on Windows
        let unix_path = validate_path_for_os("/Users/test/Documents/file.txt", true);
        assert!(unix_path.is_err());
        assert!(unix_path.unwrap_err().user_message().contains("belongs to macOS/Linux, but OpenMate is currently running on Windows"));

        // Relative paths rejected as relative
        let rel1 = validate_path_for_os("Documents/file.txt", true);
        assert!(rel1.is_err());
        assert!(rel1.unwrap_err().user_message().contains("is relative"));

        let rel2 = validate_path_for_os(r".\file.txt", true);
        assert!(rel2.is_err());
        assert!(rel2.unwrap_err().user_message().contains("is relative"));

        // Path traversal rejected
        let trav = validate_path_for_os(r"C:\Users\..\Windows\System32", true);
        assert!(trav.is_err());
        assert!(trav.unwrap_err().user_message().contains("Path traversal sequences"));
    }

    #[tokio::test]
    async fn test_write_file_explicit_overwrite_replaces_file() {
        let pe = setup_permission_engine(
            PermissionState::Allow,
            PermissionState::Allow,
            PermissionState::Allow,
        )
        .await;
        let te = ToolEngine::new(Arc::clone(&pe));

        let temp_dir = std::env::temp_dir();
        let temp_file = temp_dir.join(format!("test_openmate_overwrite_{}.txt", uuid::Uuid::new_v4()));
        std::fs::write(&temp_file, "original content").unwrap();

        let res = te
            .execute(
                Tool::WriteFile {
                    path: temp_file.clone(),
                    content: "overwritten content".to_string(),
                    overwrite: true,
                    create_copy: false,
                },
                &pe,
            )
            .await;

        assert!(res.is_ok());
        let content = std::fs::read_to_string(&temp_file).unwrap();
        assert_eq!(content, "overwritten content");

        let _ = std::fs::remove_file(&temp_file);
    }

    #[tokio::test]
    async fn test_write_file_create_copy_generates_unique_filename() {
        let pe = setup_permission_engine(
            PermissionState::Allow,
            PermissionState::Allow,
            PermissionState::Allow,
        )
        .await;
        let te = ToolEngine::new(Arc::clone(&pe));

        let temp_dir = std::env::temp_dir();
        let base_id = uuid::Uuid::new_v4();
        let original_file = temp_dir.join(format!("test_copy_{}.txt", base_id));
        std::fs::write(&original_file, "original content").unwrap();

        let res = te
            .execute(
                Tool::WriteFile {
                    path: original_file.clone(),
                    content: "copy content".to_string(),
                    overwrite: false,
                    create_copy: true,
                },
                &pe,
            )
            .await;

        assert!(res.is_ok());
        let expected_copy = temp_dir.join(format!("test_copy_{} (1).txt", base_id));
        assert!(expected_copy.exists());
        let copy_content = std::fs::read_to_string(&expected_copy).unwrap();
        assert_eq!(copy_content, "copy content");

        // Original file must remain untouched
        let orig_content = std::fs::read_to_string(&original_file).unwrap();
        assert_eq!(orig_content, "original content");

        let _ = std::fs::remove_file(&original_file);
        let _ = std::fs::remove_file(&expected_copy);
    }

    #[tokio::test]
    async fn test_write_file_existing_copies_handled_correctly() {
        let pe = setup_permission_engine(
            PermissionState::Allow,
            PermissionState::Allow,
            PermissionState::Allow,
        )
        .await;
        let te = ToolEngine::new(Arc::clone(&pe));

        let temp_dir = std::env::temp_dir();
        let base_id = uuid::Uuid::new_v4();
        let original_file = temp_dir.join(format!("test_copies_{}.txt", base_id));
        let copy_1 = temp_dir.join(format!("test_copies_{} (1).txt", base_id));

        std::fs::write(&original_file, "orig").unwrap();
        std::fs::write(&copy_1, "copy 1").unwrap();

        let res = te
            .execute(
                Tool::WriteFile {
                    path: original_file.clone(),
                    content: "copy 2 content".to_string(),
                    overwrite: false,
                    create_copy: true,
                },
                &pe,
            )
            .await;

        assert!(res.is_ok());
        let copy_2 = temp_dir.join(format!("test_copies_{} (2).txt", base_id));
        assert!(copy_2.exists());
        let copy_2_content = std::fs::read_to_string(&copy_2).unwrap();
        assert_eq!(copy_2_content, "copy 2 content");

        let _ = std::fs::remove_file(&original_file);
        let _ = std::fs::remove_file(&copy_1);
        let _ = std::fs::remove_file(&copy_2);
    }

    #[test]
    fn test_generate_non_conflicting_copy_path() {
        let temp_dir = std::env::temp_dir();
        let base_id = uuid::Uuid::new_v4();
        let path = temp_dir.join(format!("test_helper_{}.txt", base_id));

        // When path does not exist, copy path is (1)
        let copy1 = generate_non_conflicting_copy_path(&path);
        assert!(copy1.to_string_lossy().contains("(1).txt"));

        // When (1) exists, copy path is (2)
        std::fs::write(&copy1, "content").unwrap();
        let copy2 = generate_non_conflicting_copy_path(&path);
        assert!(copy2.to_string_lossy().contains("(2).txt"));

        let _ = std::fs::remove_file(&copy1);
    }

    #[tokio::test]
    async fn test_write_file_rejects_windows_path_with_clear_error_on_macos() {
        let pe = setup_permission_engine(
            PermissionState::Allow,
            PermissionState::Allow,
            PermissionState::Allow,
        )
        .await;
        let te = ToolEngine::new(Arc::clone(&pe));

        let res = te
            .execute(
                Tool::WriteFile {
                    path: PathBuf::from(r"C:\Users\Rishank\Documents\I_am_Rishank.txt"),
                    content: "Hello".to_string(),
                    overwrite: false,
                    create_copy: false,
                },
                &pe,
            )
            .await;

        assert!(res.is_err());
        let err = res.unwrap_err();
        if cfg!(unix) {
            assert!(
                err.user_message().contains("belongs to Windows, but OpenMate is currently running on macOS"),
                "Expected clear OS mismatch error, got: {}",
                err.user_message()
            );
        }
    }

    #[tokio::test]
    async fn test_write_file_permission_denied_when_off() {
        let pe = setup_permission_engine(
            PermissionState::Off,
            PermissionState::Off,
            PermissionState::Off,
        )
        .await;
        let te = ToolEngine::new(Arc::clone(&pe));

        let res = te
            .execute(
                Tool::WriteFile {
                    path: PathBuf::from("/Users/test/Documents/file.txt"),
                    content: "Hello".to_string(),
                    overwrite: false,
                    create_copy: false,
                },
                &pe,
            )
            .await;

        assert!(matches!(res, Err(OpenMateError::PermissionDenied(_))));
    }
}
