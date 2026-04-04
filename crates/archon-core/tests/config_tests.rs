use archon_core::config::{validate, default_config_path, ArchonConfig, ConfigError};

#[test]
fn empty_toml_produces_valid_defaults() {
    let config: ArchonConfig =
        toml::from_str("").expect("empty TOML should parse to defaults");

    // ApiConfig defaults
    assert_eq!(config.api.default_model, "claude-sonnet-4-6");
    assert_eq!(config.api.thinking_budget, 16384);
    assert_eq!(config.api.default_effort, "medium");
    assert_eq!(config.api.max_retries, 3);

    // IdentityConfig defaults
    assert_eq!(config.identity.mode, "clean");
    assert_eq!(config.identity.spoof_version, "2.1.89");
    assert_eq!(config.identity.spoof_entrypoint, "cli");
    assert!(config.identity.spoof_betas.is_none());
    assert!(config.identity.attestation_hook.is_none());
    assert!(!config.identity.anti_distillation);
    assert!(config.identity.workload.is_none());
    assert!(config.identity.custom.is_none());

    // ToolsConfig defaults
    assert_eq!(config.tools.bash_timeout, 120);
    assert_eq!(config.tools.bash_max_output, 102400);
    assert_eq!(config.tools.max_concurrency, 4);

    // PermissionsConfig defaults
    assert_eq!(config.permissions.mode, "default");
    assert!(config.permissions.allow_paths.is_empty());
    assert!(config.permissions.deny_paths.is_empty());

    // ContextConfig defaults
    assert!((config.context.compact_threshold - 0.80).abs() < f32::EPSILON);
    assert!(config.context.max_tokens.is_none());
    assert_eq!(config.context.preserve_recent_turns, 3);

    // MemoryConfig defaults
    assert!(config.memory.enabled);
    assert!(config.memory.db_path.is_none());

    // CostConfig defaults
    assert!((config.cost.warn_threshold - 5.0).abs() < f64::EPSILON);
    assert!((config.cost.hard_limit - 0.0).abs() < f64::EPSILON);

    // LoggingConfig defaults
    assert_eq!(config.logging.level, "info");
    assert_eq!(config.logging.max_files, 50);
    assert_eq!(config.logging.max_file_size_mb, 10);

    // SessionConfig defaults
    assert!(config.session.db_path.is_none());
    assert!(config.session.auto_resume);

    // CheckpointConfig defaults
    assert!(config.checkpoint.enabled);
    assert_eq!(config.checkpoint.max_checkpoints, 10);

    // Defaults must also pass validation
    validate(&config).expect("default config should pass validation");
}

#[test]
fn partial_toml_merges_with_defaults() {
    let toml_str = r#"
[api]
default_model = "claude-opus-4-6"
max_retries = 5
"#;
    let config: ArchonConfig =
        toml::from_str(toml_str).expect("partial TOML should parse");

    // Overridden values
    assert_eq!(config.api.default_model, "claude-opus-4-6");
    assert_eq!(config.api.max_retries, 5);

    // Non-overridden api values keep defaults
    assert_eq!(config.api.thinking_budget, 16384);
    assert_eq!(config.api.default_effort, "medium");

    // Other sections keep full defaults
    assert_eq!(config.identity.mode, "clean");
    assert_eq!(config.tools.bash_timeout, 120);
    assert_eq!(config.permissions.mode, "default");

    validate(&config).expect("partial config should pass validation");
}

#[test]
fn invalid_identity_mode_fails_validation() {
    let toml_str = r#"
[identity]
mode = "foo"
"#;
    let config: ArchonConfig =
        toml::from_str(toml_str).expect("TOML parse should succeed");
    let err = validate(&config).expect_err("invalid identity mode should fail");
    match err {
        ConfigError::ValidationError(msg) => {
            assert!(
                msg.contains("identity.mode"),
                "error should mention identity.mode, got: {msg}"
            );
        }
        other => panic!("expected ValidationError, got: {other:?}"),
    }
}

#[test]
fn bash_timeout_zero_fails_validation() {
    let toml_str = r#"
[tools]
bash_timeout = 0
"#;
    let config: ArchonConfig = toml::from_str(toml_str).expect("TOML parse ok");
    let err = validate(&config).expect_err("bash_timeout=0 should fail");
    match err {
        ConfigError::ValidationError(msg) => {
            assert!(
                msg.contains("bash_timeout"),
                "error should mention bash_timeout, got: {msg}"
            );
        }
        other => panic!("expected ValidationError, got: {other:?}"),
    }
}

#[test]
fn max_concurrency_out_of_range_fails_validation() {
    for bad_value in [0u8, 17, 255] {
        let toml_str = format!(
            "[tools]\nmax_concurrency = {bad_value}"
        );
        let config: ArchonConfig =
            toml::from_str(&toml_str).expect("TOML parse ok");
        let err = validate(&config)
            .expect_err(&format!("max_concurrency={bad_value} should fail"));
        match err {
            ConfigError::ValidationError(msg) => {
                assert!(
                    msg.contains("max_concurrency"),
                    "error should mention max_concurrency, got: {msg}"
                );
            }
            other => panic!("expected ValidationError, got: {other:?}"),
        }
    }
}

#[test]
fn compact_threshold_out_of_range_fails_validation() {
    for bad_value in [-0.1f32, 1.1, 2.0] {
        let toml_str = format!(
            "[context]\ncompact_threshold = {bad_value}"
        );
        let config: ArchonConfig =
            toml::from_str(&toml_str).expect("TOML parse ok");
        let err = validate(&config)
            .expect_err(&format!("compact_threshold={bad_value} should fail"));
        match err {
            ConfigError::ValidationError(msg) => {
                assert!(
                    msg.contains("compact_threshold"),
                    "error should mention compact_threshold, got: {msg}"
                );
            }
            other => panic!("expected ValidationError, got: {other:?}"),
        }
    }
}

#[test]
fn invalid_permissions_mode_fails_validation() {
    let toml_str = r#"
[permissions]
mode = "banana"
"#;
    let config: ArchonConfig = toml::from_str(toml_str).expect("TOML parse ok");
    let err = validate(&config).expect_err("bad permissions mode should fail");
    match err {
        ConfigError::ValidationError(msg) => {
            assert!(
                msg.contains("permissions.mode"),
                "error should mention permissions.mode, got: {msg}"
            );
        }
        other => panic!("expected ValidationError, got: {other:?}"),
    }
}

#[test]
fn extra_unknown_keys_silently_ignored() {
    let toml_str = r#"
[api]
default_model = "claude-sonnet-4-6"
some_future_field = "whatever"
another_unknown = 42

[totally_new_section]
foo = "bar"
"#;
    // This should NOT error -- forward compatibility
    let config: ArchonConfig =
        toml::from_str(toml_str).expect("unknown keys should be ignored");
    assert_eq!(config.api.default_model, "claude-sonnet-4-6");
    validate(&config).expect("config with unknown keys should validate");
}

#[test]
fn full_valid_config_parses() {
    let toml_str = r#"
[api]
default_model = "claude-opus-4-6"
thinking_budget = 32768
default_effort = "low"
max_retries = 5

[identity]
mode = "clean"
spoof_version = "3.0.0"
spoof_entrypoint = "ide"
anti_distillation = true
workload = "cron"

[tools]
bash_timeout = 60
bash_max_output = 204800
max_concurrency = 8

[permissions]
mode = "auto"
allow_paths = ["/home/user/projects"]
deny_paths = ["/etc", "/usr"]

[context]
compact_threshold = 0.90
max_tokens = 200000
preserve_recent_turns = 5

[memory]
enabled = false
db_path = "/tmp/test-memory.db"

[cost]
warn_threshold = 10.0
hard_limit = 25.0

[logging]
level = "debug"
max_files = 100
max_file_size_mb = 20

[session]
db_path = "/tmp/test-sessions.db"
auto_resume = false

[checkpoint]
enabled = false
max_checkpoints = 5
"#;
    let config: ArchonConfig =
        toml::from_str(toml_str).expect("full valid config should parse");

    assert_eq!(config.api.default_model, "claude-opus-4-6");
    assert_eq!(config.api.thinking_budget, 32768);
    assert_eq!(config.identity.mode, "clean");
    assert!(config.identity.anti_distillation);
    assert_eq!(config.tools.max_concurrency, 8);
    assert_eq!(config.permissions.mode, "auto");
    assert_eq!(config.permissions.allow_paths.len(), 1);
    assert_eq!(config.permissions.deny_paths.len(), 2);
    assert!((config.context.compact_threshold - 0.90).abs() < f32::EPSILON);
    assert_eq!(config.context.max_tokens, Some(200000));
    assert!(!config.memory.enabled);
    assert!((config.cost.hard_limit - 25.0).abs() < f64::EPSILON);
    assert_eq!(config.logging.level, "debug");
    assert!(!config.session.auto_resume);
    assert!(!config.checkpoint.enabled);

    validate(&config).expect("full valid config should pass validation");
}

#[test]
fn custom_identity_with_extra_headers_parses() {
    let toml_str = r#"
[identity]
mode = "custom"

[identity.custom]
user_agent = "my-agent/2.0"
x_app = "my-tool"

[identity.custom.extra_headers]
X-Custom-Auth = "token123"
X-Team = "backend"
"#;
    let config: ArchonConfig =
        toml::from_str(toml_str).expect("custom identity should parse");

    assert_eq!(config.identity.mode, "custom");
    let custom = config
        .identity
        .custom
        .as_ref()
        .expect("custom config should be Some");
    assert_eq!(custom.user_agent, "my-agent/2.0");
    assert_eq!(custom.x_app, "my-tool");
    let headers = custom
        .extra_headers
        .as_ref()
        .expect("extra_headers should be Some");
    assert_eq!(headers.get("X-Custom-Auth").map(String::as_str), Some("token123"));
    assert_eq!(headers.get("X-Team").map(String::as_str), Some("backend"));

    validate(&config).expect("custom identity config should validate");
}

#[test]
fn default_config_path_uses_config_dir() {
    let path = default_config_path();
    let path_str = path.to_string_lossy();
    // Should end with archon/config.toml
    assert!(
        path_str.ends_with("archon/config.toml"),
        "expected path ending with archon/config.toml, got: {path_str}"
    );
}
