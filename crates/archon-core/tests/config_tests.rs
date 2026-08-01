use archon_core::config::{ArchonConfig, ConfigError, default_config_path, validate};

#[test]
fn empty_toml_produces_valid_defaults() {
    let config: ArchonConfig = toml::from_str("").expect("empty TOML should parse to defaults");

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
    assert!(!config.identity.anti_distillation);
    assert!(config.identity.workload.is_none());
    assert!(config.identity.custom.is_none());

    // ToolsConfig defaults
    assert_eq!(config.tools.bash_timeout, 600);
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
    assert_eq!(config.context.rate_limit_pressure_tokens, Some(120_000));
    assert_eq!(config.context.rate_limit_pressure_body_bytes, Some(320_000));
    assert_eq!(config.context.large_request_retry_body_bytes, Some(320_000));
    assert_eq!(config.context.max_tool_result_bytes, 1_000_000);
    assert!(config.context.compaction_model.is_none());

    // MemoryConfig defaults
    assert!(config.memory.enabled);
    assert!(config.memory.db_path.is_none());
    assert!(
        !config
            .learning
            .agent_evolution
            .active_profile_overlay_enabled
    );

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
fn compaction_model_parses_as_explicit_context_policy() {
    let config: ArchonConfig = toml::from_str(
        r#"
[context]
compaction_model = "claude-haiku-4-5-20251001"
"#,
    )
    .expect("compaction model config should parse");

    assert_eq!(
        config.context.compaction_model.as_deref(),
        Some("claude-haiku-4-5-20251001")
    );
    validate(&config)
        .expect("explicit compaction model is resolved against provider availability at runtime");
}

#[test]
fn partial_toml_merges_with_defaults() {
    let toml_str = r#"
[api]
default_model = "claude-opus-4-7"
max_retries = 5
"#;
    let config: ArchonConfig = toml::from_str(toml_str).expect("partial TOML should parse");

    // Overridden values
    assert_eq!(config.api.default_model, "claude-opus-4-7");
    assert_eq!(config.api.max_retries, 5);

    // Non-overridden api values keep defaults
    assert_eq!(config.api.thinking_budget, 16384);
    assert_eq!(config.api.default_effort, "medium");

    // Other sections keep full defaults
    assert_eq!(config.identity.mode, "clean");
    assert_eq!(config.tools.bash_timeout, 600);
    assert_eq!(config.permissions.mode, "default");

    validate(&config).expect("partial config should pass validation");
}

#[test]
fn invalid_identity_mode_fails_validation() {
    let toml_str = r#"
[identity]
mode = "foo"
"#;
    let config: ArchonConfig = toml::from_str(toml_str).expect("TOML parse should succeed");
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
        let toml_str = format!("[tools]\nmax_concurrency = {bad_value}");
        let config: ArchonConfig = toml::from_str(&toml_str).expect("TOML parse ok");
        let err = validate(&config).expect_err(&format!("max_concurrency={bad_value} should fail"));
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
        let toml_str = format!("[context]\ncompact_threshold = {bad_value}");
        let config: ArchonConfig = toml::from_str(&toml_str).expect("TOML parse ok");
        let err =
            validate(&config).expect_err(&format!("compact_threshold={bad_value} should fail"));
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
fn max_tool_result_bytes_parses_and_too_small_fails_validation() {
    let config: ArchonConfig = toml::from_str(
        r#"
[context]
max_tool_result_bytes = 262144
"#,
    )
    .expect("context byte cap should parse");
    assert_eq!(config.context.max_tool_result_bytes, 262_144);
    validate(&config).expect("positive tool result byte cap should validate");

    let invalid: ArchonConfig = toml::from_str(
        r#"
[context]
max_tool_result_bytes = 255
"#,
    )
    .expect("small cap should parse before validation");
    let error = validate(&invalid).expect_err("too-small tool result byte cap must fail");
    assert!(error.to_string().contains("max_tool_result_bytes"));
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
    let config: ArchonConfig = toml::from_str(toml_str).expect("unknown keys should be ignored");
    assert_eq!(config.api.default_model, "claude-sonnet-4-6");
    validate(&config).expect("config with unknown keys should validate");
}

#[test]
fn full_valid_config_parses() {
    let toml_str = r#"
[api]
default_model = "claude-opus-4-7"
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
    let config: ArchonConfig = toml::from_str(toml_str).expect("full valid config should parse");

    assert_eq!(config.api.default_model, "claude-opus-4-7");
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
    let config: ArchonConfig = toml::from_str(toml_str).expect("custom identity should parse");

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
    assert_eq!(
        headers.get("X-Custom-Auth").map(String::as_str),
        Some("token123")
    );
    assert_eq!(headers.get("X-Team").map(String::as_str), Some("backend"));

    validate(&config).expect("custom identity config should validate");
}

#[test]
fn default_config_path_uses_config_dir() {
    let path = default_config_path();
    // Check path components rather than raw string so the assertion is
    // platform-agnostic (Windows uses backslashes, Unix uses forward slashes).
    let components: Vec<_> = path
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    let tail: Vec<&str> = components
        .iter()
        .rev()
        .take(2)
        .map(|s| s.as_str())
        .collect();
    assert_eq!(
        tail,
        vec!["config.toml", "archon"],
        "expected path ending with archon/config.toml, got: {}",
        path.display()
    );
}

#[test]
fn repository_project_config_template_parses() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join(".archon")
        .join("config.toml");
    let raw =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let config: ArchonConfig =
        toml::from_str(&raw).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()));

    validate(&config).unwrap_or_else(|e| panic!("validate {}: {e}", path.display()));
    assert!(config.providers.openai_codex.enabled);
    // Asserts the shape and the newest-first ordering rather than the exact
    // list. Pinning the full catalog meant every model refresh silently broke
    // this test on all three platforms: adding gpt-5.6 sol/terra/luna to the
    // shipped template left the expectation behind, and the failure said
    // nothing about models -- it just read as "the config template is broken".
    let catalog = &config.providers.openai_codex.app_server_model_catalog;
    assert!(
        catalog.len() >= 2,
        "app_server_model_catalog should list the supported models: {catalog:?}"
    );
    assert!(
        catalog.iter().all(|model| model.starts_with("gpt-")),
        "every catalog entry should be a gpt model id: {catalog:?}"
    );
    assert!(
        catalog.contains(&"gpt-5.4".to_string()),
        "the baseline gpt-5.4 entry should remain available: {catalog:?}"
    );
    assert_eq!(config.sandbox.backend, "disabled");
    assert_eq!(config.sandbox.ssh.binary, "ssh");
    assert!(!config.sandbox.openshell.provider_injection);
    assert!(
        !config
            .learning
            .agent_evolution
            .active_profile_overlay_enabled
    );
}

#[test]
fn repository_root_config_template_parses() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("config.toml");
    let raw =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let config: ArchonConfig =
        toml::from_str(&raw).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()));

    validate(&config).unwrap_or_else(|e| panic!("validate {}: {e}", path.display()));
    assert_eq!(config.sandbox.policy().unwrap().workspace_access, "ro");
    assert_eq!(config.sandbox.ssh.workspace_mode, "remote");
    assert!(!config.sandbox.openshell.host_shell_fallback);
}
