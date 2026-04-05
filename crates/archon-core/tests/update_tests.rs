//! Tests for the self-update module (`archon_core::update`).

use archon_core::update::{
    compute_sha256, is_newer, platform_asset_name, should_auto_check, UpdateConfig,
};

// ---------------------------------------------------------------------------
// is_newer — version comparison
// ---------------------------------------------------------------------------

#[test]
fn version_comparison_newer_detected() {
    assert!(is_newer("0.2.0", "0.1.0"));
    assert!(is_newer("1.0.0", "0.9.9"));
    assert!(is_newer("0.1.1", "0.1.0"));
}

#[test]
fn version_comparison_same_not_newer() {
    assert!(!is_newer("0.1.0", "0.1.0"));
    assert!(!is_newer("1.0.0", "1.0.0"));
}

#[test]
fn version_comparison_older_not_newer() {
    assert!(!is_newer("0.0.9", "0.1.0"));
    assert!(!is_newer("0.9.0", "1.0.0"));
}

#[test]
fn version_comparison_invalid_returns_false() {
    assert!(!is_newer("not-a-version", "0.1.0"));
    assert!(!is_newer("0.1.0", "not-a-version"));
    assert!(!is_newer("", ""));
}

// ---------------------------------------------------------------------------
// platform_asset_name — format
// ---------------------------------------------------------------------------

#[test]
fn platform_asset_name_format() {
    let name = platform_asset_name();
    // Must start with "archon-"
    assert!(
        name.starts_with("archon-"),
        "asset name must start with 'archon-', got: {name}"
    );
    // Must contain a known OS
    let has_os = name.contains("linux") || name.contains("darwin");
    assert!(has_os, "asset name must contain 'linux' or 'darwin', got: {name}");
    // Must contain a known arch
    let has_arch = name.contains("x86_64") || name.contains("aarch64");
    assert!(has_arch, "asset name must contain arch, got: {name}");
    // No spaces or slashes
    assert!(!name.contains(' '), "asset name must not contain spaces");
    assert!(!name.contains('/'), "asset name must not contain slashes");
}

// ---------------------------------------------------------------------------
// parse_expected_checksum — tested via local helper that replicates the logic
// ---------------------------------------------------------------------------

/// Replicate the parse logic from update.rs to test it without exposing a private fn.
fn parse_checksum_from_text(text: &str, filename: &str) -> Option<String> {
    for line in text.lines() {
        let parts: Vec<&str> = line.splitn(2, "  ").collect();
        if parts.len() == 2 && parts[1].trim() == filename {
            return Some(parts[0].trim().to_string());
        }
    }
    None
}

#[test]
fn checksum_parsed_correctly() {
    let checksum_file = "abc123  archon-linux-x86_64\ndef456  archon-darwin-aarch64\n";
    let result = parse_checksum_from_text(checksum_file, "archon-linux-x86_64");
    assert_eq!(result, Some("abc123".to_string()));
}

#[test]
fn checksum_returns_none_for_missing_filename() {
    let checksum_file = "abc123  archon-linux-x86_64\n";
    let result = parse_checksum_from_text(checksum_file, "archon-darwin-aarch64");
    assert_eq!(result, None);
}

// ---------------------------------------------------------------------------
// compute_sha256 — known values
// ---------------------------------------------------------------------------

#[test]
fn sha256_of_empty_string_correct() {
    // SHA-256("") = e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
    let result = compute_sha256(b"");
    assert_eq!(
        result,
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

#[test]
fn sha256_of_known_value_correct() {
    let result = compute_sha256(b"abc");
    // SHA-256 output is always 64 hex characters (32 bytes)
    assert_eq!(result.len(), 64, "SHA-256 hex must be 64 characters");
    assert!(
        result.chars().all(|c| c.is_ascii_hexdigit()),
        "must be valid hex"
    );
    // SHA-256("abc") starts with "ba7816bf" — well-known value
    assert!(
        result.starts_with("ba7816bf"),
        "SHA-256('abc') should start with ba7816bf, got: {result}"
    );
}

// ---------------------------------------------------------------------------
// UpdateConfig — defaults
// ---------------------------------------------------------------------------

#[test]
fn update_config_defaults() {
    let cfg = UpdateConfig::default();
    assert_eq!(cfg.channel, "stable");
    assert!(cfg.auto_check, "auto_check should default to true");
    assert_eq!(cfg.check_interval_hours, 24);
    assert!(cfg.release_url.is_none());
}

// ---------------------------------------------------------------------------
// should_auto_check — respects auto_check=false
// ---------------------------------------------------------------------------

#[test]
fn auto_check_disabled_returns_false() {
    let cfg = UpdateConfig {
        auto_check: false,
        ..UpdateConfig::default()
    };
    assert!(
        !should_auto_check(&cfg),
        "should return false when auto_check is disabled"
    );
}

#[test]
fn auto_check_enabled_no_panic() {
    // When auto_check=true and no timestamp file exists, function returns true.
    // Verify it doesn't panic regardless of file system state.
    let cfg = UpdateConfig {
        auto_check: true,
        check_interval_hours: 24,
        ..UpdateConfig::default()
    };
    let _ = should_auto_check(&cfg); // must not panic
}
