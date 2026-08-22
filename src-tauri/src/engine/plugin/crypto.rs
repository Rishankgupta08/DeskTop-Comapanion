//! # Plugin Cryptographic Verification Engine
//!
//! Handles Ed25519 signature verification and payload hashing for OpenMate plugins. [DR-040]
//!
//! ## Security rules:
//! - Public key must strictly conform to `"ed25519:<64-char-hex>"`.
//! - Verification uses `VerifyingKey::verify_strict()` to prevent signature malleability.
//! - Secret key material and raw signature bytes are never logged.

use ed25519_dalek::{Signature, VerifyingKey};
use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CryptoError {
    #[error("Invalid public key format: must start with 'ed25519:' prefix")]
    InvalidPublicKeyFormat,

    #[error("Invalid public key bytes: must be exactly 32 hex-decoded bytes")]
    InvalidPublicKeyBytes,

    #[error("Invalid signature length: must be exactly 64 bytes")]
    InvalidSignatureLength,

    #[error("Signature verification failed: payload does not match author signature")]
    SignatureVerificationFailed,
}

pub struct PluginVerifier;

impl PluginVerifier {
    /// Compute SHA-256 hash of `plugin.toml` bytes + binary bytes concatenated.
    pub fn compute_payload_hash(manifest_bytes: &[u8], binary_bytes: &[u8]) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(manifest_bytes);
        hasher.update(binary_bytes);
        hasher.finalize().into()
    }

    /// Parse an Ed25519 public key from the `"ed25519:<hex>"` string format.
    pub fn parse_public_key(public_key_hex: &str) -> Result<VerifyingKey, CryptoError> {
        let trimmed = public_key_hex.trim();
        let hex_str = match trimmed.strip_prefix("ed25519:") {
            Some(h) => h.trim(),
            None => return Err(CryptoError::InvalidPublicKeyFormat),
        };

        if hex_str.len() != 64 {
            return Err(CryptoError::InvalidPublicKeyBytes);
        }

        let raw_bytes = hex::decode(hex_str).map_err(|_| CryptoError::InvalidPublicKeyBytes)?;
        if raw_bytes.len() != 32 {
            return Err(CryptoError::InvalidPublicKeyBytes);
        }

        let bytes_array: [u8; 32] = raw_bytes
            .try_into()
            .map_err(|_| CryptoError::InvalidPublicKeyBytes)?;

        VerifyingKey::from_bytes(&bytes_array).map_err(|_| CryptoError::InvalidPublicKeyBytes)
    }

    /// Verify an Ed25519 signature over a 32-byte payload hash using strict verification.
    pub fn verify_signature(
        public_key_hex: &str,
        signature_bytes: &[u8],
        payload_hash: &[u8; 32],
    ) -> Result<(), CryptoError> {
        let verifying_key = Self::parse_public_key(public_key_hex)?;

        if signature_bytes.len() != 64 {
            return Err(CryptoError::InvalidSignatureLength);
        }

        let sig_array: [u8; 64] = signature_bytes
            .try_into()
            .map_err(|_| CryptoError::InvalidSignatureLength)?;

        let signature = Signature::from_bytes(&sig_array);

        verifying_key
            .verify_strict(payload_hash, &signature)
            .map_err(|_| CryptoError::SignatureVerificationFailed)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use rand::rngs::OsRng;

    #[test]
    fn test_verify_valid_signature() {
        let mut csprng = OsRng;
        let signing_key = SigningKey::generate(&mut csprng);
        let verifying_key = signing_key.verifying_key();
        let pubkey_str = format!("ed25519:{}", hex::encode(verifying_key.to_bytes()));

        let manifest = b"[plugin]\nid = \"test\"\n";
        let binary = b"ELF or Mach-O executable bytes";

        let hash = PluginVerifier::compute_payload_hash(manifest, binary);
        let sig = signing_key.sign(&hash);

        let result = PluginVerifier::verify_signature(&pubkey_str, &sig.to_bytes(), &hash);
        assert!(result.is_ok(), "Expected valid signature to verify successfully");
    }

    #[test]
    fn test_reject_tampered_manifest() {
        let mut csprng = OsRng;
        let signing_key = SigningKey::generate(&mut csprng);
        let verifying_key = signing_key.verifying_key();
        let pubkey_str = format!("ed25519:{}", hex::encode(verifying_key.to_bytes()));

        let manifest = b"[plugin]\nid = \"test\"\n";
        let binary = b"ELF or Mach-O executable bytes";

        let hash = PluginVerifier::compute_payload_hash(manifest, binary);
        let sig = signing_key.sign(&hash);

        // Tamper with 1 byte of the manifest
        let tampered_manifest = b"[plugin]\nid = \"evil\"\n";
        let tampered_hash = PluginVerifier::compute_payload_hash(tampered_manifest, binary);

        let result = PluginVerifier::verify_signature(&pubkey_str, &sig.to_bytes(), &tampered_hash);
        assert_eq!(
            result,
            Err(CryptoError::SignatureVerificationFailed),
            "Tampered manifest must fail verification"
        );
    }

    #[test]
    fn test_reject_tampered_binary() {
        let mut csprng = OsRng;
        let signing_key = SigningKey::generate(&mut csprng);
        let verifying_key = signing_key.verifying_key();
        let pubkey_str = format!("ed25519:{}", hex::encode(verifying_key.to_bytes()));

        let manifest = b"[plugin]\nid = \"test\"\n";
        let binary = b"original binary bytes";

        let hash = PluginVerifier::compute_payload_hash(manifest, binary);
        let sig = signing_key.sign(&hash);

        // Tamper with binary
        let tampered_binary = b"modified binary bytes";
        let tampered_hash = PluginVerifier::compute_payload_hash(manifest, tampered_binary);

        let result = PluginVerifier::verify_signature(&pubkey_str, &sig.to_bytes(), &tampered_hash);
        assert_eq!(
            result,
            Err(CryptoError::SignatureVerificationFailed),
            "Tampered binary must fail verification"
        );
    }
}
