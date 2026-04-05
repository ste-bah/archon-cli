//! Tests for TASK-CLI-228: Identity Mode Default Flip -- config layer.
//!
//! The two "default is clean" tests are expected to FAIL against the current
//! codebase (TDD red phase) because the current default is "spoof".

use archon_core::config::{ArchonConfig, validate};

// =========================================================================
// Default is clean (WILL FAIL until implementation flips the default)
// =========================================================================

#[test]
fn default_identity_mode_is_clean() {
    let config = ArchonConfig::default();
    assert_eq!(
        config.identity.mode, "clean",
        "ArchonConfig::default() identity.mode must be 'clean', got '{}'",
        config.identity.mode
    );
}

#[test]
fn empty_toml_identity_mode_is_clean() {
    let config: ArchonConfig = toml::from_str("").expect("empty TOML should parse to defaults");
    assert_eq!(
        config.identity.mode, "clean",
        "empty TOML identity.mode must default to 'clean', got '{}'",
        config.identity.mode
    );
}

// =========================================================================
// Explicit modes
// =========================================================================

#[test]
fn explicit_spoof_mode_parses() {
    let toml_str = r#"
[identity]
mode = "spoof"
"#;
    let config: ArchonConfig = toml::from_str(toml_str).expect("explicit spoof TOML should parse");
    assert_eq!(config.identity.mode, "spoof");
    validate(&config).expect("explicit spoof mode should pass validation");
}

#[test]
fn explicit_clean_mode_parses() {
    let toml_str = r#"
[identity]
mode = "clean"
"#;
    let config: ArchonConfig = toml::from_str(toml_str).expect("explicit clean TOML should parse");
    assert_eq!(config.identity.mode, "clean");
    validate(&config).expect("explicit clean mode should pass validation");
}

#[test]
fn explicit_custom_mode_parses() {
    let toml_str = r#"
[identity]
mode = "custom"

[identity.custom]
user_agent = "test-agent/1.0"
x_app = "test-app"
"#;
    let config: ArchonConfig = toml::from_str(toml_str).expect("explicit custom TOML should parse");
    assert_eq!(config.identity.mode, "custom");
    validate(&config).expect("explicit custom mode should pass validation");
}

// =========================================================================
// Invalid mode
// =========================================================================

#[test]
fn invalid_identity_mode_fails() {
    let toml_str = r#"
[identity]
mode = "invalid"
"#;
    let config: ArchonConfig =
        toml::from_str(toml_str).expect("TOML parse should succeed even with invalid mode");
    let err = validate(&config).expect_err("invalid identity mode must fail validation");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("identity.mode"),
        "error should mention identity.mode, got: {msg}"
    );
}
