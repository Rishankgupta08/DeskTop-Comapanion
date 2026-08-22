//! # openmate-cli
//!
//! Official developer and authoring CLI tool for OpenMate plugins. [DR-042]
//!
//! Subcommands:
//! - `openmate-cli plugin keygen`
//! - `openmate-cli plugin sign --key <key> --plugin <dir>`
//! - `openmate-cli plugin verify --plugin <dir>`
//! - `openmate-cli plugin package --plugin <dir> --output <file.omp>`

use clap::{Args, Parser, Subcommand};
use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use rand::rngs::OsRng;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "openmate-cli")]
#[command(about = "OpenMate Developer CLI for Plugin Signing, Packaging & Verification", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Manage, sign, verify, and package OpenMate plugins
    Plugin(PluginArgs),
}

#[derive(Args)]
struct PluginArgs {
    #[command(subcommand)]
    action: PluginSubcommands,
}

#[derive(Subcommand)]
enum PluginSubcommands {
    /// Generate an Ed25519 author keypair (public.key and private.key)
    Keygen,

    /// Sign a plugin manifest and binary with a private key
    Sign {
        /// Path to Ed25519 private key (raw bytes or hex)
        #[arg(short, long)]
        key: PathBuf,

        /// Path to the plugin directory containing plugin.toml and binary
        #[arg(short, long)]
        plugin: PathBuf,
    },

    /// Cryptographically verify a plugin directory against its signature
    Verify {
        /// Path to the plugin directory
        #[arg(short, long)]
        plugin: PathBuf,
    },

    /// Package a plugin folder into a distributable .omp archive
    Package {
        /// Path to the plugin directory
        #[arg(short, long)]
        plugin: PathBuf,

        /// Destination archive file name (e.g. plugin.omp)
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
}

fn compute_payload_hash(manifest_bytes: &[u8], binary_bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(manifest_bytes);
    hasher.update(binary_bytes);
    hasher.finalize().into()
}

fn read_binary_bytes(plugin_dir: &Path, toml_str: &str) -> anyhow::Result<Vec<u8>> {
    let manifest_val: toml::Value = toml::from_str(toml_str)?;
    let entrypoint = manifest_val
        .get("entrypoint")
        .ok_or_else(|| anyhow::anyhow!("Missing [entrypoint] table in plugin.toml"))?;

    // Try current architecture or find any declared binary in entrypoint
    let rel_path = if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        entrypoint.get("macos_arm64")
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        entrypoint.get("macos_x86_64")
    } else if cfg!(target_os = "windows") {
        entrypoint.get("windows_x86_64")
    } else {
        None
    };

    let bin_str = match rel_path.and_then(|v| v.as_str()) {
        Some(p) => p,
        None => {
            // Fallback: check any available field
            entrypoint
                .get("macos_arm64")
                .or_else(|| entrypoint.get("macos_x86_64"))
                .or_else(|| entrypoint.get("windows_x86_64"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("No binary entrypoint found in plugin.toml"))?
        }
    };

    let full_path = plugin_dir.join(bin_str);
    if !full_path.exists() {
        anyhow::bail!("Binary file '{}' not found at '{}'", bin_str, full_path.display());
    }

    let bytes = std::fs::read(&full_path)?;
    Ok(bytes)
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Plugin(args) => match args.action {
            PluginSubcommands::Keygen => {
                let mut csprng = OsRng;
                let signing_key = SigningKey::generate(&mut csprng);
                let verifying_key: VerifyingKey = signing_key.verifying_key();

                let priv_bytes = signing_key.to_bytes();
                let pub_hex = format!("ed25519:{}", hex::encode(verifying_key.to_bytes()));

                std::fs::write("private.key", priv_bytes)?;
                std::fs::write("public.key", &pub_hex)?;

                println!("Generated Ed25519 keypair successfully!");
                println!("Public Key: {}", pub_hex);
                println!("Files written: private.key (32 bytes) and public.key");
                println!("Keep private.key secret. Share public.key with users.");
            }

            PluginSubcommands::Sign { key, plugin } => {
                if !plugin.is_dir() {
                    anyhow::bail!("Plugin path '{}' is not a directory", plugin.display());
                }

                let manifest_path = plugin.join("plugin.toml");
                if !manifest_path.is_file() {
                    anyhow::bail!("plugin.toml not found in '{}'", plugin.display());
                }

                let manifest_bytes = std::fs::read(&manifest_path)?;
                let manifest_str = std::str::from_utf8(&manifest_bytes)?;
                let binary_bytes = read_binary_bytes(&plugin, manifest_str)?;

                let key_data = std::fs::read(&key)?;
                let signing_key = if key_data.len() == 32 {
                    let key_array: [u8; 32] = key_data.try_into().unwrap();
                    SigningKey::from_bytes(&key_array)
                } else if let Ok(hex_str) = std::str::from_utf8(&key_data) {
                    let trimmed = hex_str.trim().trim_start_matches("ed25519:");
                    let bytes = hex::decode(trimmed)?;
                    let key_array: [u8; 32] = bytes
                        .try_into()
                        .map_err(|_| anyhow::anyhow!("Invalid private key length"))?;
                    SigningKey::from_bytes(&key_array)
                } else {
                    anyhow::bail!("Invalid private key file format (must be 32 raw bytes or hex)");
                };

                let payload_hash = compute_payload_hash(&manifest_bytes, &binary_bytes);
                let signature = signing_key.sign(&payload_hash);

                let sig_path = plugin.join("plugin.sig");
                std::fs::write(&sig_path, signature.to_bytes())?;

                println!("Plugin signed successfully.");
                println!("Signature written to: {}", sig_path.display());
            }

            PluginSubcommands::Verify { plugin } => {
                if !plugin.is_dir() {
                    anyhow::bail!("Plugin path '{}' is not a directory", plugin.display());
                }

                let manifest_path = plugin.join("plugin.toml");
                let sig_path = plugin.join("plugin.sig");

                if !manifest_path.is_file() {
                    anyhow::bail!("plugin.toml not found in '{}'", plugin.display());
                }
                if !sig_path.is_file() {
                    println!("Signature INVALID: plugin.sig not found");
                    return Ok(());
                }

                let manifest_bytes = std::fs::read(&manifest_path)?;
                let manifest_str = std::str::from_utf8(&manifest_bytes)?;
                let binary_bytes = read_binary_bytes(&plugin, manifest_str)?;

                let manifest_val: toml::Value = toml::from_str(manifest_str)?;
                let pubkey_str = manifest_val
                    .get("plugin")
                    .and_then(|p| p.get("author_pubkey"))
                    .and_then(|k| k.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing plugin.author_pubkey in plugin.toml"))?;

                let hex_str = pubkey_str
                    .trim()
                    .strip_prefix("ed25519:")
                    .ok_or_else(|| anyhow::anyhow!("Invalid author_pubkey format (must start with 'ed25519:')"))?;

                let pubkey_bytes = hex::decode(hex_str)?;
                let pubkey_array: [u8; 32] = pubkey_bytes
                    .try_into()
                    .map_err(|_| anyhow::anyhow!("Public key must be 32 bytes"))?;

                let verifying_key = VerifyingKey::from_bytes(&pubkey_array)?;
                let sig_bytes = std::fs::read(&sig_path)?;
                let sig_array: [u8; 64] = sig_bytes
                    .try_into()
                    .map_err(|_| anyhow::anyhow!("plugin.sig must be exactly 64 bytes"))?;

                let signature = ed25519_dalek::Signature::from_bytes(&sig_array);
                let payload_hash = compute_payload_hash(&manifest_bytes, &binary_bytes);

                match verifying_key.verify_strict(&payload_hash, &signature) {
                    Ok(()) => {
                        println!("Signature valid.");
                    }
                    Err(e) => {
                        println!("Signature INVALID: {}", e);
                    }
                }
            }

            PluginSubcommands::Package { plugin, output } => {
                if !plugin.is_dir() {
                    anyhow::bail!("Plugin path '{}' is not a directory", plugin.display());
                }

                let manifest_path = plugin.join("plugin.toml");
                let sig_path = plugin.join("plugin.sig");

                if !manifest_path.is_file() {
                    anyhow::bail!("plugin.toml is required for packaging");
                }
                if !sig_path.is_file() {
                    anyhow::bail!("plugin.sig is required for packaging (run 'openmate-cli plugin sign' first)");
                }

                let out_file = output.unwrap_or_else(|| {
                    let folder_name = plugin
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("plugin");
                    PathBuf::from(format!("{}.omp", folder_name))
                });

                let file = File::create(&out_file)?;
                let mut zip = zip::ZipWriter::new(file);
                let options = zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated);

                for entry in walkdir::WalkDir::new(&plugin) {
                    let entry = entry?;
                    let path = entry.path();
                    let name = path.strip_prefix(&plugin)?;

                    if path.is_file() {
                        zip.start_file(name.to_string_lossy(), options)?;
                        let mut f = File::open(path)?;
                        let mut buffer = Vec::new();
                        f.read_to_end(&mut buffer)?;
                        zip.write_all(&buffer)?;
                    }
                }

                zip.finish()?;
                println!("Package created: {}", out_file.display());
            }
        },
    }

    Ok(())
}
