use sha2::{Digest, Sha256};

const FINGERPRINT_SALT: &str = "59cf53e54c78";

/// Compute the fingerprint for the billing header.
///
/// ```text
/// salt = "59cf53e54c78"
/// chars = msg[4] + msg[7] + msg[20] (use "0" for missing)
/// input = salt + chars + version
/// fingerprint = SHA256(input)[0:3] (first 3 hex chars)
/// ```
pub fn compute_fingerprint(first_user_message: &str, version: &str) -> String {
    let chars: Vec<u8> = first_user_message.as_bytes().to_vec();

    let c4 = chars.get(4).copied().unwrap_or(b'0') as char;
    let c7 = chars.get(7).copied().unwrap_or(b'0') as char;
    let c20 = chars.get(20).copied().unwrap_or(b'0') as char;

    let input = format!("{FINGERPRINT_SALT}{c4}{c7}{c20}{version}");
    let hash = Sha256::digest(input.as_bytes());
    let hex = hex::encode(hash);
    hex[..3].to_string()
}
