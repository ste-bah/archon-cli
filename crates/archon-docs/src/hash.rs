use sha2::{Digest, Sha256};

/// Compute SHA-256 hash of byte content, returned as hex string.
pub fn sha256_hex(content: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content);
    hex::encode(hasher.finalize())
}

/// Compute SHA-256 hash of a string, returned as hex string.
pub fn sha256_str(content: &str) -> String {
    sha256_hex(content.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sha256_deterministic() {
        let a = sha256_str("hello");
        let b = sha256_str("hello");
        assert_eq!(a, b);
    }

    #[test]
    fn test_sha256_different_inputs() {
        assert_ne!(sha256_str("hello"), sha256_str("world"));
    }

    #[test]
    fn test_sha256_empty() {
        let result = sha256_hex(b"");
        assert_eq!(result.len(), 64);
        assert_eq!(
            result,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn test_sha256_known_vector() {
        // SHA-256("abc") = ba7816bf...
        assert_eq!(
            sha256_str("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
