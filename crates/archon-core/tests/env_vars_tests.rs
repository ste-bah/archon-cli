use std::collections::HashMap;
use std::path::PathBuf;

use archon_core::config::ArchonConfig;
use archon_core::env_vars::{
    apply_env_overrides, format_doctor_env_vars, load_env_vars_from, mask_secret,
    parse_bool_env, warn_unrecognized_archon_vars, KNOWN_ARCHON_VARS,
};

// ---------------------------------------------------------------------------
// Helper: build a HashMap from key-value pairs
// ---------------------------------------------------------------------------

fn env_map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

// ===========================================================================
// parse_bool_env
// ===========================================================================

#[test]
fn parse_bool_truthy_values() {
    for val in &["1", "true", "TRUE", "True", "yes", "YES", "Yes"] {
        assert!(
            parse_bool_env(Some(val.to_string())),
            "expected true for {:?}",
            val
        );
    }
}

#[test]
fn parse_bool_falsy_values() {
    for val in &["0", "false", "FALSE", "False", "no", "NO", "No", ""] {
        assert!(
            !parse_bool_env(Some(val.to_string())),
            "expected false for {:?}",
            val
        );
    }
}

#[test]
fn parse_bool_none_is_false() {
    assert!(!parse_bool_env(None));
}

#[test]
fn parse_bool_unrecognized_string_is_false() {
    // Anything not in the truthy list should be false
    assert!(!parse_bool_env(Some("maybe".into())));
    assert!(!parse_bool_env(Some("on".into())));
    assert!(!parse_bool_env(Some("enabled".into())));
}

// ===========================================================================
// mask_secret
// ===========================================================================

#[test]
fn mask_secret_long_key() {
    let masked = mask_secret("sk-ant-1234567890abcdef");
    assert_eq!(masked, "sk-a...cdef");
}

#[test]
fn mask_secret_exactly_8_chars() {
    let masked = mask_secret("12345678");
    assert_eq!(masked, "1234...5678");
}

#[test]
fn mask_secret_short_key() {
    let masked = mask_secret("abc");
    assert_eq!(masked, "****");
}

#[test]
fn mask_secret_empty() {
    let masked = mask_secret("");
    assert_eq!(masked, "****");
}

#[test]
fn mask_secret_7_chars() {
    // Under 8 chars should mask entirely
    let masked = mask_secret("1234567");
    assert_eq!(masked, "****");
}

// ===========================================================================
// load_env_vars_from
// ===========================================================================

#[test]
fn load_from_empty_map_produces_all_none_defaults() {
    let vars = load_env_vars_from(&HashMap::new());

    assert!(vars.anthropic_api_key.is_none());
    assert!(vars.archon_api_key.is_none());
    assert!(vars.archon_oauth_token.is_none());
    assert!(vars.model.is_none());
    assert!(vars.effort.is_none());
    assert!(vars.permission_mode.is_none());
    assert!(vars.identity_mode.is_none());
    assert!(!vars.simple);
    assert!(!vars.disable_hooks);
    assert!(!vars.disable_memory);
    assert!(!vars.disable_personality);
    assert!(!vars.debug);
    assert!(vars.debug_log_dir.is_none());
    assert!(!vars.verbose);
    assert!(vars.config_dir.is_none());
    assert!(vars.data_dir.is_none());
    assert!(!vars.disable_telemetry);
}

#[test]
fn load_auth_vars() {
    let map = env_map(&[
        ("ANTHROPIC_API_KEY", "sk-ant-test123"),
        ("ARCHON_API_KEY", "sk-archon-456"),
        ("ARCHON_OAUTH_TOKEN", "oauth-token-789"),
    ]);
    let vars = load_env_vars_from(&map);

    assert_eq!(vars.anthropic_api_key.as_deref(), Some("sk-ant-test123"));
    assert_eq!(vars.archon_api_key.as_deref(), Some("sk-archon-456"));
    assert_eq!(vars.archon_oauth_token.as_deref(), Some("oauth-token-789"));
}

#[test]
fn load_model_and_behavior_vars() {
    let map = env_map(&[
        ("ARCHON_MODEL", "claude-opus-4-6"),
        ("ARCHON_EFFORT", "high"),
        ("ARCHON_PERMISSION_MODE", "auto"),
        ("ARCHON_IDENTITY_MODE", "clean"),
    ]);
    let vars = load_env_vars_from(&map);

    assert_eq!(vars.model.as_deref(), Some("claude-opus-4-6"));
    assert_eq!(vars.effort.as_deref(), Some("high"));
    assert_eq!(vars.permission_mode.as_deref(), Some("auto"));
    assert_eq!(vars.identity_mode.as_deref(), Some("clean"));
}

#[test]
fn load_boolean_feature_flags() {
    let map = env_map(&[
        ("ARCHON_SIMPLE", "1"),
        ("ARCHON_DISABLE_HOOKS", "true"),
        ("ARCHON_DISABLE_MEMORY", "yes"),
        ("ARCHON_DISABLE_PERSONALITY", "TRUE"),
    ]);
    let vars = load_env_vars_from(&map);

    assert!(vars.simple);
    assert!(vars.disable_hooks);
    assert!(vars.disable_memory);
    assert!(vars.disable_personality);
}

#[test]
fn load_boolean_feature_flags_disabled() {
    let map = env_map(&[
        ("ARCHON_SIMPLE", "0"),
        ("ARCHON_DISABLE_HOOKS", "false"),
        ("ARCHON_DISABLE_MEMORY", "no"),
        ("ARCHON_DISABLE_PERSONALITY", ""),
    ]);
    let vars = load_env_vars_from(&map);

    assert!(!vars.simple);
    assert!(!vars.disable_hooks);
    assert!(!vars.disable_memory);
    assert!(!vars.disable_personality);
}

#[test]
fn load_debug_vars() {
    let map = env_map(&[
        ("ARCHON_DEBUG", "1"),
        ("ARCHON_DEBUG_LOG_DIR", "/tmp/archon-debug"),
        ("ARCHON_VERBOSE", "yes"),
    ]);
    let vars = load_env_vars_from(&map);

    assert!(vars.debug);
    assert_eq!(vars.debug_log_dir.as_deref(), Some("/tmp/archon-debug"));
    assert!(vars.verbose);
}

#[test]
fn load_path_vars() {
    let map = env_map(&[
        ("ARCHON_CONFIG_DIR", "/custom/config"),
        ("ARCHON_DATA_DIR", "/custom/data"),
    ]);
    let vars = load_env_vars_from(&map);

    assert_eq!(vars.config_dir, Some(PathBuf::from("/custom/config")));
    assert_eq!(vars.data_dir, Some(PathBuf::from("/custom/data")));
}

#[test]
fn load_telemetry_var() {
    let map = env_map(&[("ARCHON_DISABLE_TELEMETRY", "1")]);
    let vars = load_env_vars_from(&map);
    assert!(vars.disable_telemetry);
}

// ===========================================================================
// apply_env_overrides
// ===========================================================================

#[test]
fn apply_model_override() {
    let mut config = ArchonConfig::default();
    assert_eq!(config.api.default_model, "claude-sonnet-4-6");

    let map = env_map(&[("ARCHON_MODEL", "claude-opus-4-6")]);
    let vars = load_env_vars_from(&map);
    apply_env_overrides(&mut config, &vars);

    assert_eq!(config.api.default_model, "claude-opus-4-6");
}

#[test]
fn apply_effort_override() {
    let mut config = ArchonConfig::default();
    assert_eq!(config.api.default_effort, "medium");

    let map = env_map(&[("ARCHON_EFFORT", "high")]);
    let vars = load_env_vars_from(&map);
    apply_env_overrides(&mut config, &vars);

    assert_eq!(config.api.default_effort, "high");
}

#[test]
fn apply_permission_mode_override() {
    let mut config = ArchonConfig::default();
    assert_eq!(config.permissions.mode, "default");

    let map = env_map(&[("ARCHON_PERMISSION_MODE", "auto")]);
    let vars = load_env_vars_from(&map);
    apply_env_overrides(&mut config, &vars);

    assert_eq!(config.permissions.mode, "auto");
}

#[test]
fn apply_identity_mode_override() {
    let mut config = ArchonConfig::default();
    assert_eq!(config.identity.mode, "clean");

    let map = env_map(&[("ARCHON_IDENTITY_MODE", "spoof")]);
    let vars = load_env_vars_from(&map);
    apply_env_overrides(&mut config, &vars);

    assert_eq!(config.identity.mode, "spoof");
}

#[test]
fn apply_disable_memory() {
    let mut config = ArchonConfig::default();
    assert!(config.memory.enabled);

    let map = env_map(&[("ARCHON_DISABLE_MEMORY", "1")]);
    let vars = load_env_vars_from(&map);
    apply_env_overrides(&mut config, &vars);

    assert!(!config.memory.enabled);
}

#[test]
fn apply_debug_logging_level() {
    let mut config = ArchonConfig::default();
    assert_eq!(config.logging.level, "info");

    let map = env_map(&[("ARCHON_DEBUG", "1")]);
    let vars = load_env_vars_from(&map);
    apply_env_overrides(&mut config, &vars);

    assert_eq!(config.logging.level, "debug");
}

#[test]
fn apply_verbose_logging_level() {
    let mut config = ArchonConfig::default();

    let map = env_map(&[("ARCHON_VERBOSE", "1")]);
    let vars = load_env_vars_from(&map);
    apply_env_overrides(&mut config, &vars);

    assert_eq!(config.logging.level, "trace");
}

#[test]
fn apply_debug_log_dir() {
    let mut config = ArchonConfig::default();

    let map = env_map(&[("ARCHON_DEBUG_LOG_DIR", "/tmp/archon-logs")]);
    let vars = load_env_vars_from(&map);
    apply_env_overrides(&mut config, &vars);

    // debug_log_dir override is stored but doesn't directly map to a config
    // field yet — it's exposed via ArchonEnvVars for main.rs to use.
    // The apply function stores it in the logging section if we have a field.
    // For now, verify the env var is parsed correctly.
    assert_eq!(vars.debug_log_dir.as_deref(), Some("/tmp/archon-logs"));
}

#[test]
fn apply_does_not_touch_unset_vars() {
    let mut config = ArchonConfig::default();
    let original_model = config.api.default_model.clone();
    let original_effort = config.api.default_effort.clone();
    let original_mode = config.permissions.mode.clone();

    let vars = load_env_vars_from(&HashMap::new());
    apply_env_overrides(&mut config, &vars);

    assert_eq!(config.api.default_model, original_model);
    assert_eq!(config.api.default_effort, original_effort);
    assert_eq!(config.permissions.mode, original_mode);
}

#[test]
fn verbose_overrides_debug_when_both_set() {
    let mut config = ArchonConfig::default();

    // Both ARCHON_DEBUG and ARCHON_VERBOSE set — verbose wins (trace > debug)
    let map = env_map(&[("ARCHON_DEBUG", "1"), ("ARCHON_VERBOSE", "1")]);
    let vars = load_env_vars_from(&map);
    apply_env_overrides(&mut config, &vars);

    assert_eq!(config.logging.level, "trace");
}

// ===========================================================================
// warn_unrecognized_archon_vars
// ===========================================================================

#[test]
fn unrecognized_vars_detected() {
    let map = env_map(&[
        ("ARCHON_MODEL", "opus"),
        ("ARCHON_UNKNOWN_THING", "value"),
        ("ARCHON_ANOTHER_WEIRD", "stuff"),
        ("NOT_ARCHON", "ignored"),
    ]);

    let unrecognized = warn_unrecognized_archon_vars(&map);

    assert_eq!(unrecognized.len(), 2);
    assert!(unrecognized.contains(&"ARCHON_UNKNOWN_THING".to_string()));
    assert!(unrecognized.contains(&"ARCHON_ANOTHER_WEIRD".to_string()));
}

#[test]
fn no_unrecognized_vars_when_all_known() {
    let map = env_map(&[
        ("ARCHON_MODEL", "opus"),
        ("ARCHON_DEBUG", "1"),
        ("ANTHROPIC_API_KEY", "sk-test"),
    ]);

    let unrecognized = warn_unrecognized_archon_vars(&map);
    assert!(unrecognized.is_empty());
}

#[test]
fn non_archon_vars_ignored_in_unrecognized_check() {
    let map = env_map(&[
        ("PATH", "/usr/bin"),
        ("HOME", "/home/user"),
        ("SOME_OTHER_VAR", "value"),
    ]);

    let unrecognized = warn_unrecognized_archon_vars(&map);
    assert!(unrecognized.is_empty());
}

// ===========================================================================
// format_doctor_env_vars
// ===========================================================================

#[test]
fn doctor_masks_api_keys() {
    let map = env_map(&[("ANTHROPIC_API_KEY", "sk-ant-1234567890abcdef")]);
    let vars = load_env_vars_from(&map);
    let output = format_doctor_env_vars(&vars);

    assert!(output.contains("ANTHROPIC_API_KEY"));
    // Must NOT contain the full key
    assert!(!output.contains("sk-ant-1234567890abcdef"));
    // Must contain the masked version
    assert!(output.contains("sk-a...cdef"));
}

#[test]
fn doctor_masks_archon_api_key() {
    let map = env_map(&[("ARCHON_API_KEY", "sk-archon-mykey12345678")]);
    let vars = load_env_vars_from(&map);
    let output = format_doctor_env_vars(&vars);

    assert!(!output.contains("sk-archon-mykey12345678"));
    assert!(output.contains("sk-a...5678"));
}

#[test]
fn doctor_masks_oauth_token() {
    let map = env_map(&[("ARCHON_OAUTH_TOKEN", "oauth-longtoken-abcdefgh")]);
    let vars = load_env_vars_from(&map);
    let output = format_doctor_env_vars(&vars);

    assert!(!output.contains("oauth-longtoken-abcdefgh"));
    assert!(output.contains("oaut...efgh"));
}

#[test]
fn doctor_shows_non_secret_values() {
    let map = env_map(&[
        ("ARCHON_MODEL", "claude-opus-4-6"),
        ("ARCHON_DEBUG", "1"),
    ]);
    let vars = load_env_vars_from(&map);
    let output = format_doctor_env_vars(&vars);

    assert!(output.contains("ARCHON_MODEL"));
    assert!(output.contains("claude-opus-4-6"));
    assert!(output.contains("ARCHON_DEBUG"));
}

#[test]
fn doctor_shows_telemetry_noop_message() {
    let map = env_map(&[("ARCHON_DISABLE_TELEMETRY", "1")]);
    let vars = load_env_vars_from(&map);
    let output = format_doctor_env_vars(&vars);

    assert!(output.contains("ARCHON_DISABLE_TELEMETRY"));
    assert!(output.contains("no-op"));
}

#[test]
fn doctor_shows_unset_for_missing_vars() {
    let vars = load_env_vars_from(&HashMap::new());
    let output = format_doctor_env_vars(&vars);

    // Should show the section header at minimum
    assert!(output.contains("Environment variables"));
}

// ===========================================================================
// KNOWN_ARCHON_VARS constant completeness
// ===========================================================================

#[test]
fn known_vars_list_has_correct_count() {
    // 4 auth + 4 model/behavior + 4 feature control + 4 debugging (incl ARCHON_LOG) + 2 paths + 1 telemetry = 19
    assert_eq!(KNOWN_ARCHON_VARS.len(), 19);
}

#[test]
fn known_vars_contains_all_documented_vars() {
    let expected = [
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_AUTH_TOKEN",
        "ARCHON_API_KEY",
        "ARCHON_OAUTH_TOKEN",
        "ARCHON_MODEL",
        "ARCHON_EFFORT",
        "ARCHON_PERMISSION_MODE",
        "ARCHON_IDENTITY_MODE",
        "ARCHON_SIMPLE",
        "ARCHON_DISABLE_HOOKS",
        "ARCHON_DISABLE_MEMORY",
        "ARCHON_DISABLE_PERSONALITY",
        "ARCHON_DEBUG",
        "ARCHON_DEBUG_LOG_DIR",
        "ARCHON_VERBOSE",
        "ARCHON_CONFIG_DIR",
        "ARCHON_DATA_DIR",
        "ARCHON_DISABLE_TELEMETRY",
    ];

    for var in &expected {
        assert!(
            KNOWN_ARCHON_VARS.contains(var),
            "KNOWN_ARCHON_VARS missing: {var}"
        );
    }
}

// ===========================================================================
// Edge cases
// ===========================================================================

#[test]
fn empty_string_values_treated_as_unset_for_optionals() {
    let map = env_map(&[
        ("ARCHON_MODEL", ""),
        ("ARCHON_EFFORT", ""),
        ("ARCHON_CONFIG_DIR", ""),
    ]);
    let vars = load_env_vars_from(&map);

    // Empty string for optional string/path vars should be None
    assert!(vars.model.is_none());
    assert!(vars.effort.is_none());
    assert!(vars.config_dir.is_none());
}

#[test]
fn whitespace_only_values_treated_as_unset() {
    let map = env_map(&[
        ("ARCHON_MODEL", "  "),
        ("ARCHON_EFFORT", "\t"),
    ]);
    let vars = load_env_vars_from(&map);

    assert!(vars.model.is_none());
    assert!(vars.effort.is_none());
}

#[test]
fn api_key_empty_string_is_none() {
    let map = env_map(&[("ANTHROPIC_API_KEY", "")]);
    let vars = load_env_vars_from(&map);
    assert!(vars.anthropic_api_key.is_none());
}

#[test]
fn mask_secret_non_ascii() {
    // Multibyte UTF-8 characters must not panic
    let masked = mask_secret("sk-🔑🗝️abcdefghij");
    assert!(!masked.is_empty());
    // The full secret must not appear in the masked output
    assert_ne!(masked, "sk-🔑🗝️abcdefghij");
    // Should contain the ellipsis separator
    assert!(masked.contains("..."));

    // Short non-ASCII (fewer than 8 chars) fully masked
    let masked_short = mask_secret("🔑🗝️");
    assert_eq!(masked_short, "****");
}
