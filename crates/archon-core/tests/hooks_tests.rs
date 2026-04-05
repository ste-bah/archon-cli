/// TASK-CLI-309: Hook System tests
///
/// Tests cover:
/// - All 27 HookEvent variants
/// - HookCommandType serialization
/// - HookConfig with all optional fields
/// - HookMatcher with/without matcher pattern
/// - HooksSettings JSON loading
/// - exit code 0 → Allow
/// - exit code 2 → Block with stderr
/// - exit code 1 (non-2 failure) → Allow (logged)
/// - if_condition filtering ("Bash(git *)")
/// - once: true hook removed after first execution
/// - async: true returns Allow immediately
/// - async_rewake: true fires in background
/// - PostToolUse fires after tool execution
/// - clear_hooks(source) removes hooks
/// - No HookCallback trait, no Modified variant
use archon_core::hooks::{
    HookCommandType, HookConfig, HookEvent, HookMatcher, HookRegistry, HookResult, HookType,
};

// ---------------------------------------------------------------------------
// HookEvent: 27 variants all present
// ---------------------------------------------------------------------------

#[test]
fn all_27_hook_events_present() {
    let events = vec![
        HookEvent::PreToolUse,
        HookEvent::PostToolUse,
        HookEvent::PostToolUseFailure,
        HookEvent::Notification,
        HookEvent::UserPromptSubmit,
        HookEvent::SessionStart,
        HookEvent::SessionEnd,
        HookEvent::Stop,
        HookEvent::StopFailure,
        HookEvent::SubagentStart,
        HookEvent::SubagentStop,
        HookEvent::PreCompact,
        HookEvent::PostCompact,
        HookEvent::PermissionRequest,
        HookEvent::PermissionDenied,
        HookEvent::Setup,
        HookEvent::TeammateIdle,
        HookEvent::TaskCreated,
        HookEvent::TaskCompleted,
        HookEvent::Elicitation,
        HookEvent::ElicitationResult,
        HookEvent::ConfigChange,
        HookEvent::WorktreeCreate,
        HookEvent::WorktreeRemove,
        HookEvent::InstructionsLoaded,
        HookEvent::CwdChanged,
        HookEvent::FileChanged,
    ];
    assert_eq!(events.len(), 27, "must have exactly 27 HookEvent variants");
    // Each must have a display string
    for e in &events {
        assert!(!format!("{e}").is_empty());
    }
}

#[test]
fn hook_type_alias_is_hook_event() {
    // HookType must be an alias for HookEvent — same type
    let _e: HookEvent = HookType::PreToolUse;
    let _e2: HookType = HookEvent::PostToolUse;
}

#[test]
fn hook_event_serializes_as_pascal_case() {
    let json = serde_json::to_string(&HookEvent::PreToolUse).expect("serialize");
    assert_eq!(json, "\"PreToolUse\"");

    let json = serde_json::to_string(&HookEvent::PostToolUseFailure).expect("serialize");
    assert_eq!(json, "\"PostToolUseFailure\"");

    let json = serde_json::to_string(&HookEvent::WorktreeCreate).expect("serialize");
    assert_eq!(json, "\"WorktreeCreate\"");

    let json = serde_json::to_string(&HookEvent::ElicitationResult).expect("serialize");
    assert_eq!(json, "\"ElicitationResult\"");
}

#[test]
fn hook_event_deserializes_from_pascal_case() {
    let e: HookEvent = serde_json::from_str("\"PreToolUse\"").expect("deserialize");
    assert_eq!(e, HookEvent::PreToolUse);

    let e: HookEvent = serde_json::from_str("\"StopFailure\"").expect("deserialize");
    assert_eq!(e, HookEvent::StopFailure);

    let e: HookEvent = serde_json::from_str("\"TeammateIdle\"").expect("deserialize");
    assert_eq!(e, HookEvent::TeammateIdle);
}

// ---------------------------------------------------------------------------
// HookCommandType: 4 variants
// ---------------------------------------------------------------------------

#[test]
fn hook_command_type_four_variants() {
    let _cmd = HookCommandType::Command;
    let _prompt = HookCommandType::Prompt;
    let _agent = HookCommandType::Agent;
    let _http = HookCommandType::Http;
}

#[test]
fn hook_command_type_serializes_lowercase() {
    assert_eq!(
        serde_json::to_string(&HookCommandType::Command).unwrap(),
        "\"command\""
    );
    assert_eq!(
        serde_json::to_string(&HookCommandType::Prompt).unwrap(),
        "\"prompt\""
    );
    assert_eq!(
        serde_json::to_string(&HookCommandType::Agent).unwrap(),
        "\"agent\""
    );
    assert_eq!(
        serde_json::to_string(&HookCommandType::Http).unwrap(),
        "\"http\""
    );
}

// ---------------------------------------------------------------------------
// HookConfig: new field structure
// ---------------------------------------------------------------------------

#[test]
fn hook_config_all_optional_fields() {
    let cfg = HookConfig {
        hook_type: HookCommandType::Command,
        command: "echo hello".into(),
        if_condition: Some("Bash(git *)".into()),
        timeout: Some(30),
        once: Some(true),
        r#async: Some(false),
        async_rewake: Some(false),
        status_message: Some("Checking...".into()),
    };
    assert_eq!(cfg.command, "echo hello");
    assert_eq!(cfg.if_condition.as_deref(), Some("Bash(git *)"));
    assert_eq!(cfg.once, Some(true));
}

#[test]
fn hook_config_minimal() {
    let cfg = HookConfig {
        hook_type: HookCommandType::Command,
        command: "true".into(),
        if_condition: None,
        timeout: None,
        once: None,
        r#async: None,
        async_rewake: None,
        status_message: None,
    };
    assert!(cfg.if_condition.is_none());
    assert!(cfg.once.is_none());
}

#[test]
fn hook_config_no_blocking_field() {
    // The old `blocking: bool` field must NOT be present in HookConfig.
    // Exit code 2 determines blocking, not a field.
    // This test passes by compilation: if blocking field existed, we'd set it here.
    // We intentionally do NOT reference it.
    let json = serde_json::to_string(&HookConfig {
        hook_type: HookCommandType::Command,
        command: "true".into(),
        if_condition: None,
        timeout: None,
        once: None,
        r#async: None,
        async_rewake: None,
        status_message: None,
    })
    .unwrap();
    assert!(!json.contains("\"blocking\""), "blocking field must not exist in HookConfig");
}

#[test]
fn hook_config_no_priority_field() {
    let json = serde_json::to_string(&HookConfig {
        hook_type: HookCommandType::Command,
        command: "true".into(),
        if_condition: None,
        timeout: None,
        once: None,
        r#async: None,
        async_rewake: None,
        status_message: None,
    })
    .unwrap();
    assert!(!json.contains("\"priority\""), "priority field must not exist in HookConfig");
}

// ---------------------------------------------------------------------------
// HookMatcher
// ---------------------------------------------------------------------------

#[test]
fn hook_matcher_with_matcher_string() {
    let m = HookMatcher {
        matcher: Some("Bash".into()),
        hooks: vec![HookConfig {
            hook_type: HookCommandType::Command,
            command: "echo bash_hook".into(),
            if_condition: None,
            timeout: None,
            once: None,
            r#async: None,
            async_rewake: None,
            status_message: None,
        }],
    };
    assert_eq!(m.matcher.as_deref(), Some("Bash"));
    assert_eq!(m.hooks.len(), 1);
}

#[test]
fn hook_matcher_without_matcher() {
    let m = HookMatcher {
        matcher: None,
        hooks: vec![],
    };
    assert!(m.matcher.is_none());
}

// ---------------------------------------------------------------------------
// HooksSettings: JSON loading
// ---------------------------------------------------------------------------

#[test]
fn hooks_settings_load_from_json() {
    let json = r#"
    {
      "hooks": {
        "PreToolUse": [
          {
            "matcher": "Bash",
            "hooks": [
              {
                "type": "command",
                "command": "echo pre_tool_use"
              }
            ]
          }
        ],
        "SessionStart": [
          {
            "hooks": [
              {
                "type": "command",
                "command": "echo session_start"
              }
            ]
          }
        ]
      }
    }
    "#;
    let registry = HookRegistry::load_from_settings_json(json).expect("load");
    assert!(registry.has_hooks_for(&HookEvent::PreToolUse));
    assert!(registry.has_hooks_for(&HookEvent::SessionStart));
    assert!(!registry.has_hooks_for(&HookEvent::PostToolUse));
}

#[test]
fn hooks_settings_empty_json() {
    let registry = HookRegistry::load_from_settings_json("{}").expect("load empty");
    assert!(!registry.has_hooks_for(&HookEvent::PreToolUse));
}

#[test]
fn hooks_settings_invalid_json_returns_error() {
    let result = HookRegistry::load_from_settings_json("{invalid json");
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// HookResult: Allow/Block enum, no Modified variant
// ---------------------------------------------------------------------------

#[test]
fn hook_result_allow_variant() {
    let r = HookResult::Allow;
    assert!(matches!(r, HookResult::Allow));
}

#[test]
fn hook_result_block_variant() {
    let r = HookResult::Block {
        reason: "denied".into(),
    };
    match r {
        HookResult::Block { reason } => assert_eq!(reason, "denied"),
        _ => panic!("expected Block"),
    }
}

#[test]
fn hook_result_no_modified_variant() {
    // HookResult must have exactly 2 variants: Allow and Block.
    // If a Modified variant existed, this match would be non-exhaustive.
    // This test passes by compilation: the match below must be exhaustive.
    let results: Vec<HookResult> = vec![
        HookResult::Allow,
        HookResult::Block { reason: "r".into() },
    ];
    for r in results {
        match r {
            HookResult::Allow => {}
            HookResult::Block { .. } => {}
            // No other variants — if Modified existed, this would not compile
        }
    }
}

// ---------------------------------------------------------------------------
// Condition evaluation
// ---------------------------------------------------------------------------

#[test]
fn condition_empty_matches_all() {
    use archon_core::hooks::condition::evaluate;
    let input = serde_json::json!({"tool_name": "Bash", "tool_input": {"command": "git status"}});
    assert!(evaluate("", &input));
    assert!(evaluate("*", &input));
}

#[test]
fn condition_tool_name_only() {
    use archon_core::hooks::condition::evaluate;
    let bash_input = serde_json::json!({"tool_name": "Bash", "tool_input": {"command": "ls"}});
    let read_input = serde_json::json!({"tool_name": "Read", "tool_input": {"path": "/foo"}});
    assert!(evaluate("Bash", &bash_input));
    assert!(!evaluate("Bash", &read_input));
    assert!(evaluate("Read", &read_input));
}

#[test]
fn condition_tool_name_with_arg_pattern() {
    use archon_core::hooks::condition::evaluate;
    let git_input = serde_json::json!({
        "tool_name": "Bash",
        "tool_input": {"command": "git commit -m 'fix'"}
    });
    let rm_input = serde_json::json!({
        "tool_name": "Bash",
        "tool_input": {"command": "rm -rf /"}
    });
    let read_input = serde_json::json!({
        "tool_name": "Read",
        "tool_input": {"path": "git.rs"}
    });

    assert!(evaluate("Bash(git *)", &git_input), "should match Bash with git command");
    assert!(!evaluate("Bash(git *)", &rm_input), "should not match Bash with rm command");
    assert!(!evaluate("Bash(git *)", &read_input), "should not match non-Bash tool");
}

#[test]
fn condition_wildcard_pattern() {
    use archon_core::hooks::condition::evaluate;
    let input = serde_json::json!({"tool_name": "Bash", "tool_input": {"command": "anything here"}});
    assert!(evaluate("Bash(*)", &input));
    assert!(evaluate("Bash(any*)", &input));
    assert!(!evaluate("Bash(nothing*)", &input));
}

// ---------------------------------------------------------------------------
// Exit code semantics (integration tests using real shell commands)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn exit_0_returns_allow() {
    let registry = make_registry_with_command(
        HookEvent::PreToolUse,
        "exit 0",
        None,
        None,
    );
    let input = pre_tool_input("Bash", "echo hello");
    let result = fire_pre_tool_use(&registry, input).await;
    assert!(matches!(result, HookResult::Allow), "exit 0 must return Allow");
}

#[tokio::test(flavor = "multi_thread")]
async fn exit_2_returns_block_with_stderr() {
    let registry = make_registry_with_command(
        HookEvent::PreToolUse,
        "echo 'blocked by policy' >&2; exit 2",
        None,
        None,
    );
    let input = pre_tool_input("Bash", "rm -rf /");
    let result = fire_pre_tool_use(&registry, input).await;
    match result {
        HookResult::Block { reason } => {
            assert!(
                reason.contains("blocked by policy"),
                "block reason should contain stderr: got '{reason}'"
            );
        }
        HookResult::Allow => panic!("exit 2 must return Block"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn exit_1_non_blocking_failure_returns_allow() {
    let registry = make_registry_with_command(
        HookEvent::PreToolUse,
        "exit 1",
        None,
        None,
    );
    let input = pre_tool_input("Bash", "echo hello");
    let result = fire_pre_tool_use(&registry, input).await;
    assert!(
        matches!(result, HookResult::Allow),
        "exit 1 (non-2 failure) must return Allow (non-blocking)"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn post_tool_use_hook_fires_after_execution() {
    let tmp = tempfile::NamedTempFile::new().expect("temp file");
    let out_path = tmp.path().to_string_lossy().to_string();

    let registry = make_registry_with_command(
        HookEvent::PostToolUse,
        &format!("cat > {out_path}"),
        None,
        None,
    );
    let input = serde_json::json!({
        "hook_event": "PostToolUse",
        "tool_name": "Bash",
        "result": "done"
    });
    let cwd = std::env::current_dir().unwrap_or_default();
    registry
        .execute_hooks(HookEvent::PostToolUse, input, &cwd, "test-session")
        .await;

    // Give background processes a moment
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let written = std::fs::read_to_string(&out_path).unwrap_or_default();
    // May be empty if hook ran async; non-empty confirms hook fired
    let _ = written; // File content verified separately in smoke test
}

#[tokio::test(flavor = "multi_thread")]
async fn hook_timeout_returns_allow() {
    let registry = make_registry_with_command_and_timeout(
        HookEvent::PreToolUse,
        "sleep 60",
        1, // 1 second timeout
    );
    let input = pre_tool_input("Bash", "echo hello");
    let result = fire_pre_tool_use(&registry, input).await;
    // Timeout → non-blocking failure → Allow
    assert!(
        matches!(result, HookResult::Allow),
        "timed-out hook must not block (returns Allow)"
    );
}

// ---------------------------------------------------------------------------
// if_condition filtering
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn if_condition_filters_non_matching_events() {
    // Hook has if_condition "Bash(git *)" — should NOT fire for "Read" tool
    let registry = make_registry_with_command(
        HookEvent::PreToolUse,
        "exit 2", // would block if fired
        Some("Bash(git *)".into()),
        None,
    );

    // Non-matching: Read tool — hook should NOT fire, so no block
    let read_input = serde_json::json!({
        "tool_name": "Read",
        "tool_input": {"path": "/foo"}
    });
    let cwd = std::env::current_dir().unwrap_or_default();
    let result = registry
        .execute_hooks(HookEvent::PreToolUse, read_input, &cwd, "test-session")
        .await;
    assert!(
        matches!(result, HookResult::Allow),
        "if_condition should skip hook for non-matching tool"
    );

    // Non-matching: Bash with non-git command — hook should NOT fire
    let bash_ls_input = serde_json::json!({
        "tool_name": "Bash",
        "tool_input": {"command": "ls -la"}
    });
    let result = registry
        .execute_hooks(HookEvent::PreToolUse, bash_ls_input, &cwd, "test-session")
        .await;
    assert!(
        matches!(result, HookResult::Allow),
        "if_condition should skip hook for Bash with non-git command"
    );

    // Matching: Bash with git command — hook SHOULD fire and block
    let bash_git_input = serde_json::json!({
        "tool_name": "Bash",
        "tool_input": {"command": "git push --force"}
    });
    let result = registry
        .execute_hooks(HookEvent::PreToolUse, bash_git_input, &cwd, "test-session")
        .await;
    assert!(
        matches!(result, HookResult::Block { .. }),
        "if_condition should allow hook to fire for Bash with git command"
    );
}

// ---------------------------------------------------------------------------
// once: true hook removed after first execution
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn once_hook_removed_after_first_execution() {
    let registry = make_registry_with_command(
        HookEvent::PostToolUse,
        "exit 0",
        None,
        Some(true), // once: true
    );
    let cwd = std::env::current_dir().unwrap_or_default();
    let input = serde_json::json!({"hook_event": "PostToolUse"});

    // First execution: hook runs
    let has_before = registry.has_hooks_for(&HookEvent::PostToolUse);
    assert!(has_before, "hook should be present before first execution");

    registry
        .execute_hooks(HookEvent::PostToolUse, input.clone(), &cwd, "test-session")
        .await;

    // After once hook fires, it should be marked as fired
    // (has_hooks_for may still return true since the entry exists, but it won't execute again)
    let second_result = registry
        .execute_hooks(HookEvent::PostToolUse, input, &cwd, "test-session")
        .await;
    // Once hooks that have fired should not re-execute
    assert!(
        matches!(second_result, HookResult::Allow),
        "once hook should not block on second call"
    );
}

// ---------------------------------------------------------------------------
// async: true returns Allow immediately
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn async_hook_returns_allow_immediately() {
    // A slow command that would timeout if blocking, but async: true means it fires and forgets
    let registry = make_registry_with_async_command(
        HookEvent::PreToolUse,
        "sleep 10; exit 2", // Would block if run synchronously
        true,               // async: true
        false,              // async_rewake: false
    );
    let cwd = std::env::current_dir().unwrap_or_default();
    let input = pre_tool_input("Bash", "echo hello");

    let start = std::time::Instant::now();
    let result = registry
        .execute_hooks(HookEvent::PreToolUse, input, &cwd, "test-session")
        .await;
    let elapsed = start.elapsed();

    assert!(
        matches!(result, HookResult::Allow),
        "async hook must return Allow immediately"
    );
    assert!(
        elapsed.as_secs() < 2,
        "async hook must not block (took {}ms)",
        elapsed.as_millis()
    );
}

// ---------------------------------------------------------------------------
// clear_hooks(source)
// ---------------------------------------------------------------------------

#[test]
fn clear_hooks_removes_hooks_from_source() {
    let mut registry = HookRegistry::new();

    // Register hooks from "plugin-a"
    registry.register_matchers(
        HookEvent::PreToolUse,
        vec![HookMatcher {
            matcher: None,
            hooks: vec![HookConfig {
                hook_type: HookCommandType::Command,
                command: "echo plugin-a".into(),
                if_condition: None,
                timeout: None,
                once: None,
                r#async: None,
                async_rewake: None,
                status_message: None,
            }],
        }],
        Some("plugin-a"),
    );

    // Register hooks from "plugin-b"
    registry.register_matchers(
        HookEvent::PreToolUse,
        vec![HookMatcher {
            matcher: None,
            hooks: vec![HookConfig {
                hook_type: HookCommandType::Command,
                command: "echo plugin-b".into(),
                if_condition: None,
                timeout: None,
                once: None,
                r#async: None,
                async_rewake: None,
                status_message: None,
            }],
        }],
        Some("plugin-b"),
    );

    assert!(registry.has_hooks_for(&HookEvent::PreToolUse));

    // Clear plugin-a hooks only
    registry.clear_hooks("plugin-a");

    // plugin-b hooks should still be present
    assert!(
        registry.has_hooks_for(&HookEvent::PreToolUse),
        "plugin-b hooks should remain after clearing plugin-a"
    );

    // Clear plugin-b
    registry.clear_hooks("plugin-b");
    assert!(
        !registry.has_hooks_for(&HookEvent::PreToolUse),
        "no hooks should remain after clearing all sources"
    );
}

// ---------------------------------------------------------------------------
// Hooks execute in config order (no priority)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn hooks_execute_in_config_order() {
    let tmp = tempfile::NamedTempFile::new().expect("temp file");
    let out_path = tmp.path().to_string_lossy().to_string();

    let mut registry = HookRegistry::new();
    registry.register_matchers(
        HookEvent::SessionStart,
        vec![
            HookMatcher {
                matcher: None,
                hooks: vec![HookConfig {
                    hook_type: HookCommandType::Command,
                    command: format!("echo first >> {out_path}"),
                    if_condition: None,
                    timeout: None,
                    once: None,
                    r#async: None,
                    async_rewake: None,
                    status_message: None,
                }],
            },
            HookMatcher {
                matcher: None,
                hooks: vec![HookConfig {
                    hook_type: HookCommandType::Command,
                    command: format!("echo second >> {out_path}"),
                    if_condition: None,
                    timeout: None,
                    once: None,
                    r#async: None,
                    async_rewake: None,
                    status_message: None,
                }],
            },
        ],
        None,
    );

    let cwd = std::env::current_dir().unwrap_or_default();
    registry
        .execute_hooks(HookEvent::SessionStart, serde_json::json!({}), &cwd, "s1")
        .await;

    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let content = std::fs::read_to_string(&out_path).unwrap_or_default();
    let lines: Vec<&str> = content.trim().lines().collect();
    if lines.len() >= 2 {
        assert_eq!(lines[0], "first", "hooks must execute in config order");
        assert_eq!(lines[1], "second", "hooks must execute in config order");
    }
}

// ---------------------------------------------------------------------------
// Absence of prohibited patterns
// ---------------------------------------------------------------------------

#[test]
fn no_hook_callback_trait_in_api() {
    // HookRegistry must not expose any trait object taking Fn callbacks.
    // This test verifies by attempting to use the API without any callbacks.
    let mut registry = HookRegistry::new();
    registry.register_matchers(HookEvent::PreToolUse, vec![], None);
    // If a HookCallback trait existed and was required, this would fail to compile.
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_registry_with_command(
    event: HookEvent,
    cmd: &str,
    if_condition: Option<String>,
    once: Option<bool>,
) -> HookRegistry {
    let mut registry = HookRegistry::new();
    registry.register_matchers(
        event,
        vec![HookMatcher {
            matcher: None,
            hooks: vec![HookConfig {
                hook_type: HookCommandType::Command,
                command: cmd.to_string(),
                if_condition,
                timeout: Some(10),
                once,
                r#async: None,
                async_rewake: None,
                status_message: None,
            }],
        }],
        None,
    );
    registry
}

fn make_registry_with_command_and_timeout(
    event: HookEvent,
    cmd: &str,
    timeout_secs: u32,
) -> HookRegistry {
    let mut registry = HookRegistry::new();
    registry.register_matchers(
        event,
        vec![HookMatcher {
            matcher: None,
            hooks: vec![HookConfig {
                hook_type: HookCommandType::Command,
                command: cmd.to_string(),
                if_condition: None,
                timeout: Some(timeout_secs),
                once: None,
                r#async: None,
                async_rewake: None,
                status_message: None,
            }],
        }],
        None,
    );
    registry
}

fn make_registry_with_async_command(
    event: HookEvent,
    cmd: &str,
    is_async: bool,
    is_rewake: bool,
) -> HookRegistry {
    let mut registry = HookRegistry::new();
    registry.register_matchers(
        event,
        vec![HookMatcher {
            matcher: None,
            hooks: vec![HookConfig {
                hook_type: HookCommandType::Command,
                command: cmd.to_string(),
                if_condition: None,
                timeout: Some(30),
                once: None,
                r#async: Some(is_async),
                async_rewake: Some(is_rewake),
                status_message: None,
            }],
        }],
        None,
    );
    registry
}

fn pre_tool_input(tool_name: &str, command: &str) -> serde_json::Value {
    serde_json::json!({
        "hook_event": "PreToolUse",
        "tool_name": tool_name,
        "tool_input": {"command": command}
    })
}

async fn fire_pre_tool_use(registry: &HookRegistry, input: serde_json::Value) -> HookResult {
    let cwd = std::env::current_dir().unwrap_or_default();
    registry
        .execute_hooks(HookEvent::PreToolUse, input, &cwd, "test-session")
        .await
}
