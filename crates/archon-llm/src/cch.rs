//! CCH (Claude Code Hash) request signing.
//!
//! Computes an xxhash64 fingerprint of the serialised request body and
//! embeds it in the x-anthropic-billing-header. The server uses the hash
//! to verify the request originated from a legitimate Claude Code client.
//!
//! Algorithm and seed match claurst's reference implementation
//! (third-party Rust port of Claude Code) at
//! /tmp/claurst/src-rust/crates/api/src/cch.rs.

use xxhash_rust::xxh64::xxh64;

const CCH_SEED: u64 = 0x6E52_736A_C806_831E;
const CCH_MASK: u64 = 0xF_FFFF; // 5 hex digits

/// Compute the 5-hex-digit CCH hash for `body`. Format: `cch=<5hex>`.
pub fn compute_cch(body: &[u8]) -> String {
    let hash = xxh64(body, CCH_SEED) & CCH_MASK;
    format!("cch={hash:05x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cch_format_is_5_hex_with_prefix() {
        let h = compute_cch(b"test");
        assert!(h.starts_with("cch="));
        assert_eq!(h.len(), 9); // "cch=" + 5 hex
    }

    #[test]
    fn cch_is_deterministic() {
        assert_eq!(compute_cch(b"same body"), compute_cch(b"same body"));
    }

    #[test]
    fn cch_differs_for_different_bodies() {
        assert_ne!(compute_cch(b"body a"), compute_cch(b"body b"));
    }

    #[test]
    fn cch_known_vector() {
        // xxh64(b"test body", 0x6E52736AC806831E) & 0xFFFFF
        assert_eq!(compute_cch(b"test body"), "cch=08b7e");
    }
}
