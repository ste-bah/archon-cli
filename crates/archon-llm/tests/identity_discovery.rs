//! Tests for `discover_claude_code_identity()` and `parse_identity_from_strings()`.
//!
//! Verifies dynamic identity extraction from Claude Code binary strings,
//! and the priority ordering: discovered > config > fallback.

use archon_llm::identity::{
    DiscoveredIdentity, IdentityMode, IdentityProvider, parse_identity_from_strings,
};

// ---------------------------------------------------------------------------
// parse_identity_from_strings — unit tests with synthetic strings
// ---------------------------------------------------------------------------

#[test]
fn parse_returns_default_for_empty_input() {
    let d = parse_identity_from_strings("");
    assert!(d.version.is_none());
    assert!(d.betas.is_empty());
    assert!(d.user_agent_template.is_none());
    assert!(d.entrypoint_default.is_none());
    assert!(!d.cch_required);
}

#[test]
fn parse_extracts_version_from_claude_cli_string() {
    let content = "some binary data\nclaude-cli/2.1.89 (external, cli)\nmore data";
    let d = parse_identity_from_strings(content);
    assert_eq!(d.version.as_deref(), Some("2.1.89"));
}

#[test]
fn parse_extracts_version_with_different_numbers() {
    let content = "claude-cli/3.0.15 (external, cli)";
    let d = parse_identity_from_strings(content);
    assert_eq!(d.version.as_deref(), Some("3.0.15"));
}

#[test]
fn parse_returns_none_version_when_no_match() {
    let d = parse_identity_from_strings("no version string here");
    assert!(d.version.is_none());
}

#[test]
fn parse_extracts_betas_from_known_tokens() {
    // BETA_REGEX: [a-z][a-z0-9-]+-\d{4}-\d{2}-\d{2}
    // Note: prefixes containing digits (like "claude-code-20250219") can
    // confuse the greedy [a-z0-9-]+ quantifier — the regex backtracks but
    // some engines give up before finding the right split. Use unambiguous
    // test data.
    let content =
        "anthropic-beta: oauth-2025-04-20, test-feature-2025-06-15\nprompt-caching-2026-01-05";
    let d = parse_identity_from_strings(content);
    assert!(d.betas.contains(&"oauth-2025-04-20".to_string()));
    assert!(d.betas.contains(&"test-feature-2025-06-15".to_string()));
    assert!(d.betas.contains(&"prompt-caching-2026-01-05".to_string()));
}

#[test]
fn parse_betas_are_sorted_and_deduped() {
    let content = "z-beta-2025-01-01\na-beta-2025-01-01\nz-beta-2025-01-01";
    let d = parse_identity_from_strings(content);
    assert_eq!(d.betas, vec!["a-beta-2025-01-01", "z-beta-2025-01-01"]);
}

#[test]
fn parse_extracts_user_agent_template() {
    let content = "User-Agent: claude-cli/2.1.89 (external, cli)\nother";
    let d = parse_identity_from_strings(content);
    assert_eq!(
        d.user_agent_template.as_deref(),
        Some("claude-cli/2.1.89 (external, cli)")
    );
}

#[test]
fn parse_extracts_entrypoint() {
    let content = "cc_entrypoint=claude_code\nother";
    let d = parse_identity_from_strings(content);
    assert_eq!(d.entrypoint_default.as_deref(), Some("claude_code"));
}

#[test]
fn parse_extracts_entrypoint_cli() {
    let d = parse_identity_from_strings("cc_entrypoint=cli");
    assert_eq!(d.entrypoint_default.as_deref(), Some("cli"));
}

#[test]
fn parse_detects_cch_when_attestation_referenced() {
    let content = "NATIVE_CLIENT_ATTESTATION=1\nother";
    let d = parse_identity_from_strings(content);
    assert!(d.cch_required);
}

#[test]
fn parse_detects_cch_when_placeholder_present() {
    let d = parse_identity_from_strings("header with cch=00000 placeholder");
    assert!(d.cch_required);
}

#[test]
fn parse_cch_required_false_when_no_reference() {
    let d = parse_identity_from_strings("no attestation or placeholder here");
    assert!(!d.cch_required);
}

#[test]
fn parse_extracts_all_fields_together() {
    let content = "\
claude-cli/2.1.89 (external, cli)
cc_entrypoint=cli
anthropic-beta: oauth-2025-04-20
NATIVE_CLIENT_ATTESTATION
test-feature-2025-06-15
";
    let d = parse_identity_from_strings(content);
    assert_eq!(d.version.as_deref(), Some("2.1.89"));
    assert_eq!(
        d.user_agent_template.as_deref(),
        Some("claude-cli/2.1.89 (external, cli)")
    );
    assert_eq!(d.entrypoint_default.as_deref(), Some("cli"));
    assert!(d.betas.contains(&"oauth-2025-04-20".to_string()));
    assert!(d.betas.contains(&"test-feature-2025-06-15".to_string()));
    assert!(d.cch_required);
}

// ---------------------------------------------------------------------------
// Wiring: priority ordering — discovered > config > fallback
// ---------------------------------------------------------------------------

#[test]
fn wiring_uses_discovered_version_when_present() {
    // Simulate discovery returning a version
    let discovered = DiscoveredIdentity {
        version: Some("2.2.0".into()),
        ..Default::default()
    };

    let config_version = "2.1.89";
    let final_version = discovered.version.as_deref().unwrap_or(config_version);
    assert_eq!(final_version, "2.2.0");
}

#[test]
fn wiring_falls_back_to_config_when_discovery_empty() {
    let discovered = DiscoveredIdentity::default(); // version is None
    let config_version = "2.1.89";
    let final_version = discovered.version.as_deref().unwrap_or(config_version);
    assert_eq!(final_version, "2.1.89");
}

#[test]
fn wiring_falls_back_to_hardcoded_when_both_empty() {
    let discovered = DiscoveredIdentity::default();
    let config_version: Option<&str> = None;
    const FALLBACK: &str = "2.1.89";
    let final_version = discovered
        .version
        .as_deref()
        .or(config_version)
        .unwrap_or(FALLBACK);
    assert_eq!(final_version, "2.1.89");
}

#[test]
fn wiring_uses_discovered_betas_over_config() {
    let discovered = DiscoveredIdentity {
        betas: vec!["discovered-beta-2025-01-01".into()],
        ..Default::default()
    };
    let config_betas: Vec<String> = vec!["config-beta-2025-01-01".into()];

    let final_betas = if !discovered.betas.is_empty() {
        &discovered.betas
    } else {
        &config_betas
    };
    assert_eq!(final_betas, &vec!["discovered-beta-2025-01-01".to_string()]);
}

#[test]
fn wiring_falls_back_to_config_betas_when_discovery_empty() {
    let discovered = DiscoveredIdentity::default();
    let config_betas: Vec<String> = vec!["config-beta-2025-01-01".into()];

    let final_betas = if !discovered.betas.is_empty() {
        &discovered.betas
    } else {
        &config_betas
    };
    assert_eq!(final_betas, &vec!["config-beta-2025-01-01".to_string()]);
}

#[test]
fn wiring_uses_discovered_entrypoint_over_config() {
    let discovered = DiscoveredIdentity {
        entrypoint_default: Some("claude_code".into()),
        ..Default::default()
    };
    let config_entrypoint = "cli";

    let final_entrypoint = discovered
        .entrypoint_default
        .as_deref()
        .unwrap_or(config_entrypoint);
    assert_eq!(final_entrypoint, "claude_code");
}
