//! Tests for identity field resolvers — priority: discovered > config > default.
//!
//! v0.1.16: completes the spoof auto-discovery loop. Tests cover version,
//! entrypoint, user-agent resolution with source tags, and memoization.

use archon_llm::identity::{parse_identity_from_strings, resolve_entrypoint, resolve_user_agent};

// Since resolve_spoof_version internally calls discover_claude_code_identity()
// which scans the actual filesystem, these tests exercise the priority logic
// directly via parse_identity_from_strings + manual priority ordering.

// ---------------------------------------------------------------------------
// Version resolution
// ---------------------------------------------------------------------------

#[test]
fn version_uses_strings_when_present() {
    // Binary strings contain a claude-cli/X.Y.Z literal — preferred path.
    let content = "claude-cli/3.0.0 (external, cli)\nother data";
    let d = parse_identity_from_strings(content);
    let config_version = "2.1.89";

    let resolved = d.version.as_deref().unwrap_or(config_version);
    assert_eq!(resolved, "3.0.0");
}

#[test]
fn version_falls_back_to_config_when_strings_empty() {
    // No version in strings — config should win.
    let d = parse_identity_from_strings("no version here");
    let config_version = "2.1.89";

    let resolved = d.version.as_deref().unwrap_or(config_version);
    assert_eq!(resolved, "2.1.89");
}

#[test]
fn version_falls_back_to_default_when_both_empty() {
    // Neither discovery nor config provides a version.
    let d = parse_identity_from_strings("");
    let config_version: Option<&str> = None;
    const FALLBACK: &str = "2.1.89";

    let resolved = d.version.as_deref().or(config_version).unwrap_or(FALLBACK);
    assert_eq!(resolved, FALLBACK);
}

// ---------------------------------------------------------------------------
// Entrypoint resolution
// ---------------------------------------------------------------------------

#[test]
fn entrypoint_uses_discovered_when_present() {
    let content = "cc_entrypoint=claude_code\nother";
    let d = parse_identity_from_strings(content);
    let config_ep = "cli";

    let resolved = d.entrypoint_default.as_deref().unwrap_or(config_ep);
    assert_eq!(resolved, "claude_code");
}

#[test]
fn entrypoint_falls_back_to_config_when_not_discovered() {
    let d = parse_identity_from_strings("no entrypoint here");
    let config_ep = "cli";

    let resolved = d.entrypoint_default.as_deref().unwrap_or(config_ep);
    assert_eq!(resolved, "cli");
}

#[test]
fn entrypoint_uses_cli_when_discovered() {
    // Verify "cli" entrypoint is extracted correctly.
    let content = "cc_entrypoint=cli\nother";
    let d = parse_identity_from_strings(content);
    let config_ep = "claude_code";

    let resolved = d.entrypoint_default.as_deref().unwrap_or(config_ep);
    assert_eq!(resolved, "cli");
}

// ---------------------------------------------------------------------------
// User-agent resolution
// ---------------------------------------------------------------------------

#[test]
fn user_agent_uses_discovered_template_when_available() {
    let content = "claude-cli/2.1.119 (external, cli)\nother";
    let d = parse_identity_from_strings(content);

    // When template is found, it should be used verbatim.
    let (ua, source) = resolve_user_agent("2.1.119", d.user_agent_template.as_deref());
    assert_eq!(ua, "claude-cli/2.1.119 (external, cli)");
    assert_eq!(source, "discovered");
}

#[test]
fn user_agent_composes_from_version_when_no_template() {
    // No UA template in binary — compose from version.
    let (ua, source) = resolve_user_agent("2.1.89", None);
    assert_eq!(ua, "claude-cli/2.1.89 (external, cli)");
    assert_eq!(source, "composed");
}

#[test]
fn user_agent_composes_from_different_version() {
    let (ua, source) = resolve_user_agent("3.5.0", None);
    assert_eq!(ua, "claude-cli/3.5.0 (external, cli)");
    assert_eq!(source, "composed");
}

// ---------------------------------------------------------------------------
// Source label verification
// ---------------------------------------------------------------------------

#[test]
fn resolve_entrypoint_returns_correct_source_labels() {
    // Use the public resolve_entrypoint which calls discover_claude_code_identity().
    // When Claude Code is not installed, source will be "config".
    let (_ep, source) = resolve_entrypoint("cli");
    // If Claude Code happens to be installed, source could be "discovered".
    // Test that source is one of the valid labels.
    assert!(
        source == "config" || source == "discovered",
        "expected 'config' or 'discovered', got '{source}'"
    );
}

// ---------------------------------------------------------------------------
// Memoization invariant
// ---------------------------------------------------------------------------

#[test]
fn memoized_discovery_returns_same_reference() {
    // discover_claude_code_identity() is memoised via OnceLock.
    // Two calls must return the same static reference.
    let first = archon_llm::identity::discover_claude_code_identity() as *const _;
    let second = archon_llm::identity::discover_claude_code_identity() as *const _;
    assert_eq!(
        first, second,
        "discover_claude_code_identity should return the same reference (memoised)"
    );
}

#[test]
fn memoized_discovery_returns_consistent_data() {
    // Multiple calls return identical field values.
    let d1 = archon_llm::identity::discover_claude_code_identity();
    let d2 = archon_llm::identity::discover_claude_code_identity();
    assert_eq!(d1.version, d2.version);
    assert_eq!(d1.betas, d2.betas);
    assert_eq!(d1.entrypoint_default, d2.entrypoint_default);
    assert_eq!(d1.user_agent_template, d2.user_agent_template);
    assert_eq!(d1.cch_required, d2.cch_required);
}
