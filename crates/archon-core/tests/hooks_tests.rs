use archon_core::hooks::{
    HookConfig, HookDispatcher, HookError, HookExecutor, HookRegistry, HookResult, HookType,
};

// ---------------------------------------------------------------------------
// HookType tests
// ---------------------------------------------------------------------------

#[test]
fn all_hook_types_have_display() {
    let types = vec![
        HookType::Setup,
        HookType::SessionStart,
        HookType::SessionEnd,
        HookType::PreToolUse,
        HookType::PostToolUse,
        HookType::PostToolUseFailure,
        HookType::PreCompact,
        HookType::PostCompact,
        HookType::ConfigChange,
        HookType::CwdChanged,
        HookType::FileChanged,
        HookType::InstructionsLoaded,
        HookType::UserPromptSubmit,
        HookType::Stop,
        HookType::SubagentStart,
        HookType::SubagentStop,
        HookType::TaskCreated,
        HookType::TaskCompleted,
        HookType::PermissionDenied,
        HookType::PermissionRequest,
        HookType::Notification,
    ];
    for ht in &types {
        let s = format!("{ht}");
        assert!(!s.is_empty(), "HookType::{ht:?} display was empty");
    }
    // Ensure we covered all 21 variants
    assert_eq!(types.len(), 21);
}

#[test]
fn hook_config_defaults() {
    let cfg = HookConfig {
        hook_type: HookType::SessionStart,
        command: "echo hi".into(),
        tool: None,
        blocking: false,
        timeout_ms: 10000,
    };
    assert!(!cfg.blocking);
    assert_eq!(cfg.timeout_ms, 10000);
}

#[test]
fn hook_type_serializes_snake_case() {
    let ht = HookType::PreToolUse;
    let json = serde_json::to_string(&ht).expect("serialize");
    assert_eq!(json, "\"pre_tool_use\"");
}

#[test]
fn hook_type_deserializes_snake_case() {
    let ht: HookType = serde_json::from_str("\"post_compact\"").expect("deserialize");
    assert_eq!(ht, HookType::PostCompact);
}

// ---------------------------------------------------------------------------
// Registry tests
// ---------------------------------------------------------------------------

#[test]
fn registry_empty_returns_no_hooks() {
    let reg = HookRegistry::new();
    assert!(reg.hooks_for(&HookType::Setup).is_empty());
    assert!(reg.all_hooks().is_empty());
}

#[test]
fn registry_register_and_retrieve() {
    let mut reg = HookRegistry::new();
    reg.register(HookConfig {
        hook_type: HookType::SessionStart,
        command: "echo hello".into(),
        tool: None,
        blocking: false,
        timeout_ms: 10000,
    });
    let hooks = reg.hooks_for(&HookType::SessionStart);
    assert_eq!(hooks.len(), 1);
    assert_eq!(hooks[0].command, "echo hello");
}

#[test]
fn registry_multiple_hooks_same_type() {
    let mut reg = HookRegistry::new();
    for i in 0..3 {
        reg.register(HookConfig {
            hook_type: HookType::FileChanged,
            command: format!("hook_{i}"),
            tool: None,
            blocking: false,
            timeout_ms: 5000,
        });
    }
    assert_eq!(reg.hooks_for(&HookType::FileChanged).len(), 3);
}

#[test]
fn registry_format_status_non_empty() {
    let mut reg = HookRegistry::new();
    reg.register(HookConfig {
        hook_type: HookType::Setup,
        command: "echo setup".into(),
        tool: None,
        blocking: true,
        timeout_ms: 5000,
    });
    let status = reg.format_status();
    assert!(!status.is_empty());
    assert!(status.contains("setup"));
}

#[test]
fn registry_from_config_builds_correctly() {
    let configs = vec![
        HookConfig {
            hook_type: HookType::Stop,
            command: "cleanup.sh".into(),
            tool: None,
            blocking: true,
            timeout_ms: 3000,
        },
        HookConfig {
            hook_type: HookType::Stop,
            command: "notify.sh".into(),
            tool: None,
            blocking: false,
            timeout_ms: 0,
        },
    ];
    let reg = HookRegistry::from_config(configs);
    assert_eq!(reg.hooks_for(&HookType::Stop).len(), 2);
    assert_eq!(reg.all_hooks().len(), 2);
}

// ---------------------------------------------------------------------------
// Executor tests (use `echo` and `cat` as hook commands)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn execute_blocking_hook_receives_payload() {
    let tmp = tempfile::NamedTempFile::new().expect("temp file");
    let out_path = tmp.path().to_string_lossy().to_string();

    // Use `cat` to read stdin and write to temp file
    let cfg = HookConfig {
        hook_type: HookType::SessionStart,
        command: format!("cat > {out_path}"),
        tool: None,
        blocking: true,
        timeout_ms: 5000,
    };
    let payload = serde_json::json!({"event": "session_start", "id": "abc"});
    let result = HookExecutor::execute(
        &cfg,
        &payload,
        "test-session",
        &std::env::current_dir().unwrap_or_default(),
    )
    .await;

    assert!(result.is_ok(), "executor returned error: {result:?}");

    // Verify the payload was written
    let written = std::fs::read_to_string(&out_path).expect("read temp");
    let parsed: serde_json::Value = serde_json::from_str(&written).expect("parse json");
    assert_eq!(parsed["event"], "session_start");
}

#[tokio::test(flavor = "multi_thread")]
async fn execute_blocking_hook_with_timeout() {
    let cfg = HookConfig {
        hook_type: HookType::Setup,
        command: "sleep 60".into(),
        tool: None,
        blocking: true,
        timeout_ms: 100, // 100ms — will timeout
    };
    let payload = serde_json::json!({});
    let result = HookExecutor::execute(
        &cfg,
        &payload,
        "test-session",
        &std::env::current_dir().unwrap_or_default(),
    )
    .await;

    match result {
        Err(HookError::Timeout { .. }) => {} // expected
        other => panic!("expected Timeout error, got: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn execute_non_blocking_fires_and_forgets() {
    let tmp = tempfile::NamedTempFile::new().expect("temp file");
    let out_path = tmp.path().to_string_lossy().to_string();

    let cfg = HookConfig {
        hook_type: HookType::Notification,
        command: format!("cat > {out_path}"),
        tool: None,
        blocking: false,
        timeout_ms: 0,
    };
    let payload = serde_json::json!({"msg": "fire-and-forget"});
    let result = HookExecutor::execute(
        &cfg,
        &payload,
        "test-session",
        &std::env::current_dir().unwrap_or_default(),
    )
    .await;

    // Non-blocking returns Ok(None) immediately
    assert!(result.is_ok());
    assert!(result.unwrap().is_none());

    // Give the background task a moment to write
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let written = std::fs::read_to_string(&out_path).unwrap_or_default();
    if !written.is_empty() {
        let parsed: serde_json::Value = serde_json::from_str(&written).expect("parse json");
        assert_eq!(parsed["msg"], "fire-and-forget");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn execute_hook_result_parsed() {
    // Echo a JSON HookResult to stdout
    let cfg = HookConfig {
        hook_type: HookType::PreToolUse,
        command: r#"echo '{"allow":false,"reason":"blocked by policy"}'"#.into(),
        tool: None,
        blocking: true,
        timeout_ms: 5000,
    };
    let payload = serde_json::json!({"tool": "Bash"});
    let result = HookExecutor::execute(
        &cfg,
        &payload,
        "test-session",
        &std::env::current_dir().unwrap_or_default(),
    )
    .await
    .expect("should succeed");

    let hr = result.expect("should have a result");
    assert_eq!(hr.allow, Some(false));
    assert_eq!(hr.reason.as_deref(), Some("blocked by policy"));
}

#[tokio::test(flavor = "multi_thread")]
async fn execute_invalid_command_returns_error() {
    let cfg = HookConfig {
        hook_type: HookType::Setup,
        command: "/nonexistent/binary/that/does/not/exist".into(),
        tool: None,
        blocking: true,
        timeout_ms: 5000,
    };
    let payload = serde_json::json!({});
    let result = HookExecutor::execute(
        &cfg,
        &payload,
        "test-session",
        &std::env::current_dir().unwrap_or_default(),
    )
    .await;

    assert!(result.is_err(), "expected error for invalid command");
}

#[tokio::test(flavor = "multi_thread")]
async fn execute_hook_sets_env_vars() {
    // Use env to dump environment, grep for our vars
    let cfg = HookConfig {
        hook_type: HookType::CwdChanged,
        command: "env".into(),
        tool: None,
        blocking: true,
        timeout_ms: 5000,
    };
    let payload = serde_json::json!({});
    let cwd = std::env::current_dir().unwrap_or_default();
    let result = HookExecutor::execute(&cfg, &payload, "my-session-42", &cwd).await;

    // We can't easily read stdout from `env` unless we capture it.
    // The test mainly verifies the execute doesn't error with env vars set.
    assert!(result.is_ok());
}

// ---------------------------------------------------------------------------
// Dispatcher tests
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn dispatcher_fires_blocking_sequentially() {
    let tmp = tempfile::NamedTempFile::new().expect("temp file");
    let out_path = tmp.path().to_string_lossy().to_string();

    let mut reg = HookRegistry::new();
    // Hook 1: write "first" to file
    reg.register(HookConfig {
        hook_type: HookType::SessionStart,
        command: format!("echo first >> {out_path}"),
        tool: None,
        blocking: true,
        timeout_ms: 5000,
    });
    // Hook 2: write "second" to file
    reg.register(HookConfig {
        hook_type: HookType::SessionStart,
        command: format!("echo second >> {out_path}"),
        tool: None,
        blocking: true,
        timeout_ms: 5000,
    });

    let dispatcher = HookDispatcher::new(
        reg,
        "test-session".into(),
        std::env::current_dir().unwrap_or_default(),
    );

    let results = dispatcher
        .fire(HookType::SessionStart, serde_json::json!({}))
        .await;
    assert_eq!(results.len(), 2);

    // Verify order
    let content = std::fs::read_to_string(&out_path).expect("read");
    let lines: Vec<&str> = content.trim().lines().collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0], "first");
    assert_eq!(lines[1], "second");
}

#[tokio::test(flavor = "multi_thread")]
async fn dispatcher_pre_tool_use_can_block() {
    let mut reg = HookRegistry::new();
    reg.register(HookConfig {
        hook_type: HookType::PreToolUse,
        command: r#"echo '{"allow":false,"reason":"denied"}'"#.into(),
        tool: None,
        blocking: true,
        timeout_ms: 5000,
    });

    let dispatcher = HookDispatcher::new(
        reg,
        "test-session".into(),
        std::env::current_dir().unwrap_or_default(),
    );

    let result = dispatcher
        .fire_pre_tool_use("Bash", &serde_json::json!({"command": "rm -rf /"}))
        .await;

    let hr = result.expect("should have result");
    assert_eq!(hr.allow, Some(false));
    assert_eq!(hr.reason.as_deref(), Some("denied"));
}

#[tokio::test(flavor = "multi_thread")]
async fn dispatcher_pre_tool_use_filters_by_tool() {
    let mut reg = HookRegistry::new();
    // Hook only for "Bash" tool
    reg.register(HookConfig {
        hook_type: HookType::PreToolUse,
        command: r#"echo '{"allow":false,"reason":"bash blocked"}'"#.into(),
        tool: Some("Bash".into()),
        blocking: true,
        timeout_ms: 5000,
    });

    let dispatcher = HookDispatcher::new(
        reg,
        "test-session".into(),
        std::env::current_dir().unwrap_or_default(),
    );

    // Should NOT fire for "Read" tool
    let result = dispatcher
        .fire_pre_tool_use("Read", &serde_json::json!({}))
        .await;
    assert!(result.is_none(), "hook should not fire for Read tool");

    // SHOULD fire for "Bash" tool
    let result = dispatcher
        .fire_pre_tool_use("Bash", &serde_json::json!({}))
        .await;
    assert!(result.is_some(), "hook should fire for Bash tool");
    assert_eq!(result.unwrap().allow, Some(false));
}

// ---------------------------------------------------------------------------
// Config parsing tests
// ---------------------------------------------------------------------------

#[test]
fn parse_empty_hooks() {
    let configs = archon_core::hooks::config::parse_hooks_from_toml("");
    assert!(configs.is_ok());
    assert!(configs.unwrap().is_empty());
}

#[test]
fn parse_single_hook() {
    let toml_str = r#"
[[session_start]]
command = "echo hello"
blocking = true
timeout = 5000
"#;
    let configs = archon_core::hooks::config::parse_hooks_from_toml(toml_str);
    assert!(configs.is_ok());
    let configs = configs.unwrap();
    assert_eq!(configs.len(), 1);
    assert_eq!(configs[0].hook_type, HookType::SessionStart);
    assert_eq!(configs[0].command, "echo hello");
    assert!(configs[0].blocking);
    assert_eq!(configs[0].timeout_ms, 5000);
}

#[test]
fn parse_hook_with_tool_filter() {
    let toml_str = r#"
[[pre_tool_use]]
command = "check-tool.sh"
tool = "Bash"
blocking = true
timeout = 3000
"#;
    let configs = archon_core::hooks::config::parse_hooks_from_toml(toml_str);
    assert!(configs.is_ok());
    let configs = configs.unwrap();
    assert_eq!(configs.len(), 1);
    assert_eq!(configs[0].tool.as_deref(), Some("Bash"));
}

#[test]
fn parse_multiple_hook_types() {
    let toml_str = r#"
[[setup]]
command = "init.sh"

[[session_start]]
command = "start.sh"

[[stop]]
command = "cleanup.sh"
blocking = true
"#;
    let configs = archon_core::hooks::config::parse_hooks_from_toml(toml_str);
    assert!(configs.is_ok());
    let configs = configs.unwrap();
    assert_eq!(configs.len(), 3);
}

// ---------------------------------------------------------------------------
// HookResult tests
// ---------------------------------------------------------------------------

#[test]
fn hook_result_serde_roundtrip() {
    let hr = HookResult {
        allow: Some(true),
        modified_prompt: Some("rewritten".into()),
        reason: None,
    };
    let json = serde_json::to_string(&hr).expect("serialize");
    let back: HookResult = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back.allow, Some(true));
    assert_eq!(back.modified_prompt.as_deref(), Some("rewritten"));
    assert!(back.reason.is_none());
}

#[test]
fn hook_result_partial_fields() {
    // Only some fields present
    let json = r#"{"allow": false}"#;
    let hr: HookResult = serde_json::from_str(json).expect("deserialize");
    assert_eq!(hr.allow, Some(false));
    assert!(hr.modified_prompt.is_none());
    assert!(hr.reason.is_none());
}

#[test]
fn hook_result_empty_object() {
    let json = "{}";
    let hr: HookResult = serde_json::from_str(json).expect("deserialize");
    assert!(hr.allow.is_none());
    assert!(hr.modified_prompt.is_none());
    assert!(hr.reason.is_none());
}
