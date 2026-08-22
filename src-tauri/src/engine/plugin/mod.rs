//! # OpenMate Plugin Engine
//!
//! Subsystem providing sandboxed out-of-process Rust plugin execution with
//! Ed25519 signature verification, trust registry management, and capability checks. [DR-039, DR-040, DR-041]

pub mod crypto;
pub mod host;
pub mod loader;
pub mod manifest;
pub mod sandbox;
pub mod trust;

pub use crypto::{CryptoError, PluginVerifier};
pub use host::{PluginCallError, PluginHost, ToolCallResult};
pub use loader::{LoadedPlugin, PluginLoadError, PluginLoader};
pub use manifest::{ManifestError, PluginManifest, PluginMeta, PluginTool};
pub use trust::{AuthorEntry, TrustError, TrustLevel, TrustRegistry, TrustStore};
