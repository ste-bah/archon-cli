use std::path::PathBuf;

use rand::RngCore;
use sha2::{Digest, Sha256};

/// Generate a random 32-byte token encoded as 64 lowercase hex characters.
pub fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// Constant-time comparison of two tokens using SHA-256 hashing of both
/// values to prevent timing-based side-channel attacks.
pub fn validate_token(expected: &str, provided: &str) -> bool {
    if provided.is_empty() {
        return false;
    }
    let expected_hash = Sha256::digest(expected.as_bytes());
    let provided_hash = Sha256::digest(provided.as_bytes());
    // Compare as fixed-size arrays — same length, no short-circuit possible
    expected_hash == provided_hash
}

/// Returns the default path for the remote token file:
/// `~/.config/archon/remote-token`
pub fn token_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from(".config"))
        .join("archon")
        .join("remote-token")
}

/// Load the remote bearer token from disk. If the file does not exist,
/// generate a new token, persist it, and return it.
pub fn load_or_create_token() -> anyhow::Result<String> {
    let path = token_path();
    if path.exists() {
        let token = std::fs::read_to_string(&path)?.trim().to_string();
        if !token.is_empty() {
            return Ok(token);
        }
    }
    // Generate and persist a new token
    let token = generate_token();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, &token)?;
    Ok(token)
}
