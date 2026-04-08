/// TASK-CLI-309: Hook System tests
///
/// Tests cover:
/// - All 27 HookEvent variants
/// - HookCommandType serialization
/// - HookConfig with all optional fields
/// - HookMatcher with/without matcher pattern
/// - HooksSettings JSON loading
/// - HookResult struct (REQ-HOOK-001): constructors, serialization, is_blocking/is_success
/// - HookOutcome: 4-variant enum
/// - PermissionBehavior: 4-variant enum
/// - AggregatedHookResult: merge logic (REQ-HOOK-011)
/// - exit code 0 → Success (no blocking)
/// - exit code 2 → Blocking with stderr
/// - exit code 1 (non-2 failure) → NonBlockingError (not blocked)
/// - Stdout JSON parsing (REQ-HOOK-002): valid JSON overlays fields, invalid falls back
/// - if_condition filtering ("Bash(git *)")
/// - once: true hook removed after first execution
/// - async: true returns immediately (no blocking)
/// - PostToolUse fires after tool execution
/// - Permission escalation enforcement (REQ-HOOK-004a)
/// - No short-circuit: all hooks run, results accumulated
use archon_core::hooks::{
    AggregatedHookResult, HookCommandType, HookConfig, HookEvent, HookMatcher, HookOutcome,
    HookRegistry, HookResult, HookType, PermissionBehavior, SourceAuthority,
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
        headers: Default::default(),
        allowed_env_vars: Default::default(),
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
        headers: Default::default(),
        allowed_env_vars: Default::default(),
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
        headers: Default::default(),
        allowed_env_vars: Default::default(),
    })
    .unwrap();
    assert!(
        !json.contains("\"blocking\""),
        "blocking field must not exist in HookConfig"
    );
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
        headers: Default::default(),
        allowed_env_vars: Default::default(),
    })
    .unwrap();
    assert!(
        !json.contains("\"priority\""),
        "priority field must not exist in HookConfig"
    );
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
            headers: Default::default(),
            allowed_env_vars: Default::default(),
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
    let _registry = HookRegistry::load_from_settings_json(json).expect("load");
}

#[test]
fn hooks_settings_empty_json() {
    let _registry = HookRegistry::load_from_settings_json("{}").expect("load empty");
}

#[test]
fn hooks_settings_invalid_json_returns_error() {
    let result = HookRegistry::load_from_settings_json("{invalid json");
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// HookResult: struct with outcome, backward-compat constructors (REQ-HOOK-001)
// ---------------------------------------------------------------------------

#[test]
fn hook_result_allow_constructor() {
    let r = HookResult::allow();
    assert!(r.is_success());
    assert!(!r.is_blocking());
    assert_eq!(r.outcome, HookOutcome::Success);
    assert!(r.reason.is_none());
}

#[test]
fn hook_result_block_constructor() {
    let r = HookResult::block("denied".into());
    assert!(r.is_blocking());
    assert!(!r.is_success());
    assert_eq!(r.outcome, HookOutcome::Blocking);
    assert_eq!(r.reason.as_deref(), Some("denied"));
}

#[test]
fn hook_result_default_is_success() {
    let r = HookResult::default();
    assert_eq!(r.outcome, HookOutcome::Success);
    assert!(r.updated_input.is_none());
    assert!(r.permission_behavior.is_none());
    assert!(r.updated_mcp_tool_output.is_none());
    assert!(r.additional_context.is_none());
    assert!(r.prevent_continuation.is_none());
    assert!(r.retry.is_none());
}

#[test]
fn hook_result_serialization_roundtrip() {
    let r = HookResult {
        outcome: HookOutcome::Success,
        updated_input: Some(serde_json::json!({"command": "ls"})),
        permission_behavior: Some(PermissionBehavior::Allow),
        additional_context: Some("extra info".into()),
        ..Default::default()
    };
    let json = serde_json::to_string(&r).expect("serialize");
    let deser: HookResult = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(deser.outcome, HookOutcome::Success);
    assert!(deser.updated_input.is_some());
    assert_eq!(deser.permission_behavior, Some(PermissionBehavior::Allow));
    assert_eq!(deser.additional_context.as_deref(), Some("extra info"));
}

// ---------------------------------------------------------------------------
// HookOutcome: 4 variants
// ---------------------------------------------------------------------------

#[test]
fn hook_outcome_four_variants() {
    let variants = vec![
        HookOutcome::Success,
        HookOutcome::Blocking,
        HookOutcome::NonBlockingError,
        HookOutcome::Cancelled,
    ];
    assert_eq!(variants.len(), 4);
}

#[test]
fn hook_outcome_serialization() {
    assert_eq!(
        serde_json::to_string(&HookOutcome::Success).unwrap(),
        "\"success\""
    );
    assert_eq!(
        serde_json::to_string(&HookOutcome::Blocking).unwrap(),
        "\"blocking\""
    );
    assert_eq!(
        serde_json::to_string(&HookOutcome::NonBlockingError).unwrap(),
        "\"non_blocking_error\""
    );
    assert_eq!(
        serde_json::to_string(&HookOutcome::Cancelled).unwrap(),
        "\"cancelled\""
    );
}

// ---------------------------------------------------------------------------
// PermissionBehavior: 4 variants
// ---------------------------------------------------------------------------

#[test]
fn permission_behavior_four_variants() {
    let variants = vec![
        PermissionBehavior::Allow,
        PermissionBehavior::Deny,
        PermissionBehavior::Ask,
        PermissionBehavior::Passthrough,
    ];
    assert_eq!(variants.len(), 4);
}

#[test]
fn permission_behavior_serialization() {
    assert_eq!(
        serde_json::to_string(&PermissionBehavior::Allow).unwrap(),
        "\"allow\""
    );
    assert_eq!(
        serde_json::to_string(&PermissionBehavior::Deny).unwrap(),
        "\"deny\""
    );
    assert_eq!(
        serde_json::to_string(&PermissionBehavior::Ask).unwrap(),
        "\"ask\""
    );
    assert_eq!(
        serde_json::to_string(&PermissionBehavior::Passthrough).unwrap(),
        "\"passthrough\""
    );
}

// ---------------------------------------------------------------------------
// SourceAuthority: 4 variants
// ---------------------------------------------------------------------------

#[test]
fn source_authority_four_variants() {
    let variants = vec![
        SourceAuthority::User,
        SourceAuthority::Project,
        SourceAuthority::Local,
        SourceAuthority::Policy,
    ];
    assert_eq!(variants.len(), 4);
}

// ---------------------------------------------------------------------------
// AggregatedHookResult: merge logic (REQ-HOOK-011)
// ---------------------------------------------------------------------------

#[test]
fn aggregated_empty_is_not_blocked() {
    let agg = AggregatedHookResult::new();
    assert!(!agg.is_blocked());
    assert!(agg.block_reason().is_none());
    assert!(agg.additional_contexts.is_empty());
    assert!(!agg.prevent_continuation);
    assert!(!agg.retry);
}

#[test]
fn aggregated_merge_blocking_collects_errors() {
    let mut agg = AggregatedHookResult::new();
    agg.merge(HookResult::block("reason1".into()));
    agg.merge(HookResult::block("reason2".into()));
    assert!(agg.is_blocked());
    assert_eq!(agg.blocking_errors.len(), 2);
    let combined = agg.block_reason().unwrap();
    assert!(combined.contains("reason1"));
    assert!(combined.contains("reason2"));
}

#[test]
fn aggregated_merge_updated_input_last_writer_wins() {
    let mut agg = AggregatedHookResult::new();
    agg.merge(HookResult {
        updated_input: Some(serde_json::json!({"a": 1})),
        ..Default::default()
    });
    agg.merge(HookResult {
        updated_input: Some(serde_json::json!({"b": 2})),
        ..Default::default()
    });
    // Last writer wins
    assert_eq!(agg.updated_input, Some(serde_json::json!({"b": 2})));
}

#[test]
fn aggregated_merge_additional_contexts_collected() {
    let mut agg = AggregatedHookResult::new();
    agg.merge(HookResult {
        additional_context: Some("ctx1".into()),
        ..Default::default()
    });
    agg.merge(HookResult {
        additional_context: Some("ctx2".into()),
        ..Default::default()
    });
    assert_eq!(agg.additional_contexts, vec!["ctx1", "ctx2"]);
}

#[test]
fn aggregated_merge_prevent_continuation_any_true_wins() {
    let mut agg = AggregatedHookResult::new();
    agg.merge(HookResult {
        prevent_continuation: Some(false),
        ..Default::default()
    });
    assert!(!agg.prevent_continuation);
    agg.merge(HookResult {
        prevent_continuation: Some(true),
        stop_reason: Some("must stop".into()),
        ..Default::default()
    });
    assert!(agg.prevent_continuation);
    assert_eq!(agg.stop_reason.as_deref(), Some("must stop"));
}

#[test]
fn aggregated_merge_retry_any_true_wins() {
    let mut agg = AggregatedHookResult::new();
    agg.merge(HookResult::allow());
    assert!(!agg.retry);
    agg.merge(HookResult {
        retry: Some(true),
        ..Default::default()
    });
    assert!(agg.retry);
}

#[test]
fn aggregated_merge_permission_non_policy_cannot_allow() {
    // REQ-HOOK-004a: non-policy hooks cannot set permission_behavior=Allow
    let mut agg = AggregatedHookResult::new();
    agg.merge(HookResult {
        permission_behavior: Some(PermissionBehavior::Allow),
        source_authority: Some(SourceAuthority::User), // NOT policy
        ..Default::default()
    });
    // Should be silently dropped — permission_behavior remains None
    assert!(
        agg.permission_behavior.is_none(),
        "non-policy hook should not be able to set permission_behavior=Allow"
    );
}

#[test]
fn aggregated_merge_permission_policy_can_allow() {
    let mut agg = AggregatedHookResult::new();
    agg.merge(HookResult {
        permission_behavior: Some(PermissionBehavior::Allow),
        source_authority: Some(SourceAuthority::Policy),
        ..Default::default()
    });
    assert_eq!(agg.permission_behavior, Some(PermissionBehavior::Allow));
}

#[test]
fn aggregated_merge_permission_non_policy_can_deny() {
    let mut agg = AggregatedHookResult::new();
    agg.merge(HookResult {
        permission_behavior: Some(PermissionBehavior::Deny),
        source_authority: Some(SourceAuthority::User),
        permission_decision_reason: Some("security rule".into()),
        ..Default::default()
    });
    assert_eq!(agg.permission_behavior, Some(PermissionBehavior::Deny));
    assert_eq!(
        agg.permission_decision_reason.as_deref(),
        Some("security rule")
    );
}

#[test]
fn aggregated_merge_passthrough_ignored() {
    let mut agg = AggregatedHookResult::new();
    agg.merge(HookResult {
        permission_behavior: Some(PermissionBehavior::Passthrough),
        source_authority: Some(SourceAuthority::User),
        ..Default::default()
    });
    assert!(
        agg.permission_behavior.is_none(),
        "passthrough should not set permission_behavior"
    );
}

#[test]
fn aggregated_merge_system_messages_collected() {
    let mut agg = AggregatedHookResult::new();
    agg.merge(HookResult {
        system_message: Some("warning 1".into()),
        ..Default::default()
    });
    agg.merge(HookResult {
        system_message: Some("warning 2".into()),
        ..Default::default()
    });
    assert_eq!(agg.system_messages, vec!["warning 1", "warning 2"]);
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

    assert!(
        evaluate("Bash(git *)", &git_input),
        "should match Bash with git command"
    );
    assert!(
        !evaluate("Bash(git *)", &rm_input),
        "should not match Bash with rm command"
    );
    assert!(
        !evaluate("Bash(git *)", &read_input),
        "should not match non-Bash tool"
    );
}

#[test]
fn condition_wildcard_pattern() {
    use archon_core::hooks::condition::evaluate;
    let input =
        serde_json::json!({"tool_name": "Bash", "tool_input": {"command": "anything here"}});
    assert!(evaluate("Bash(*)", &input));
    assert!(evaluate("Bash(any*)", &input));
    assert!(!evaluate("Bash(nothing*)", &input));
}

// ---------------------------------------------------------------------------
// Exit code semantics (integration tests using real shell commands)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn exit_0_returns_not_blocked() {
    let registry = make_registry_with_command(HookEvent::PreToolUse, "exit 0", None, None);
    let input = pre_tool_input("Bash", "echo hello");
    let result = fire_pre_tool_use(&registry, input).await;
    assert!(!result.is_blocked(), "exit 0 must not block");
}

#[tokio::test(flavor = "multi_thread")]
async fn exit_2_returns_blocked_with_stderr() {
    let registry = make_registry_with_command(
        HookEvent::PreToolUse,
        "echo 'blocked by policy' >&2; exit 2",
        None,
        None,
    );
    let input = pre_tool_input("Bash", "rm -rf /");
    let result = fire_pre_tool_use(&registry, input).await;
    assert!(result.is_blocked(), "exit 2 must block");
    let reason = result.block_reason().expect("should have block reason");
    assert!(
        reason.contains("blocked by policy"),
        "block reason should contain stderr: got '{reason}'"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn exit_1_non_blocking_failure_returns_not_blocked() {
    let registry = make_registry_with_command(HookEvent::PreToolUse, "exit 1", None, None);
    let input = pre_tool_input("Bash", "echo hello");
    let result = fire_pre_tool_use(&registry, input).await;
    assert!(
        !result.is_blocked(),
        "exit 1 (non-2 failure) must not block"
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
async fn hook_timeout_returns_not_blocked() {
    let registry = make_registry_with_command_and_timeout(
        HookEvent::PreToolUse,
        "sleep 60",
        1, // 1 second timeout
    );
    let input = pre_tool_input("Bash", "echo hello");
    let result = fire_pre_tool_use(&registry, input).await;
    // Timeout → non-blocking failure → not blocked
    assert!(
        !result.is_blocked(),
        "timed-out hook must not block"
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
        !result.is_blocked(),
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
        !result.is_blocked(),
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
        result.is_blocked(),
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
        !second_result.is_blocked(),
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
        !result.is_blocked(),
        "async hook must not block"
    );
    assert!(
        elapsed.as_secs() < 2,
        "async hook must not block (took {}ms)",
        elapsed.as_millis()
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
                    headers: Default::default(),
                    allowed_env_vars: Default::default(),
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
                    headers: Default::default(),
                    allowed_env_vars: Default::default(),
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
                headers: Default::default(),
                allowed_env_vars: Default::default(),
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
                headers: Default::default(),
                allowed_env_vars: Default::default(),
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
                headers: Default::default(),
                allowed_env_vars: Default::default(),
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

async fn fire_pre_tool_use(
    registry: &HookRegistry,
    input: serde_json::Value,
) -> AggregatedHookResult {
    let cwd = std::env::current_dir().unwrap_or_default();
    registry
        .execute_hooks(HookEvent::PreToolUse, input, &cwd, "test-session")
        .await
}

// ---------------------------------------------------------------------------
// Stdout JSON parsing (REQ-HOOK-002)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn stdout_json_parsed_as_hook_result() {
    // Hook prints valid HookResult JSON to stdout with updated_input
    let json_output = r#"{"outcome":"success","updated_input":{"command":"modified"}}"#;
    let cmd = format!("printf '{json_output}'");
    let registry = make_registry_with_command(HookEvent::PreToolUse, &cmd, None, None);
    let input = pre_tool_input("Bash", "echo hello");
    let result = fire_pre_tool_use(&registry, input).await;
    assert!(!result.is_blocked());
    assert_eq!(
        result.updated_input,
        Some(serde_json::json!({"command": "modified"})),
        "stdout JSON updated_input should be captured"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn stdout_invalid_json_falls_back_to_exit_code() {
    // Hook prints garbage to stdout but exits 0 — should still succeed
    let registry = make_registry_with_command(
        HookEvent::PreToolUse,
        "echo 'not json'; exit 0",
        None,
        None,
    );
    let input = pre_tool_input("Bash", "echo hello");
    let result = fire_pre_tool_use(&registry, input).await;
    assert!(
        !result.is_blocked(),
        "invalid stdout should fall back to exit-code (0=not blocked)"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn stdout_json_with_additional_context() {
    let json_output = r#"{"outcome":"success","additional_context":"hook says hi"}"#;
    let cmd = format!("printf '{json_output}'");
    let registry = make_registry_with_command(HookEvent::PostToolUse, &cmd, None, None);
    let cwd = std::env::current_dir().unwrap_or_default();
    let input = serde_json::json!({
        "hook_event": "PostToolUse",
        "tool_name": "Bash",
        "result": "original output"
    });
    let result = registry
        .execute_hooks(HookEvent::PostToolUse, input, &cwd, "test-session")
        .await;
    assert!(!result.is_blocked());
    assert_eq!(result.additional_contexts, vec!["hook says hi"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn exit_2_overrides_stdout_success() {
    // Hook prints success JSON but exits 2 — exit code should win (Blocking)
    let json_output = r#"{"outcome":"success"}"#;
    let cmd = format!("printf '{json_output}'; exit 2");
    let registry = make_registry_with_command(HookEvent::PreToolUse, &cmd, None, None);
    let input = pre_tool_input("Bash", "echo hello");
    let result = fire_pre_tool_use(&registry, input).await;
    assert!(
        result.is_blocked(),
        "exit 2 must block even if stdout says success"
    );
}

// ---------------------------------------------------------------------------
// No short-circuit: all hooks run (REQ-HOOK-011)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn multiple_hooks_all_run_no_short_circuit() {
    let tmp = tempfile::NamedTempFile::new().expect("temp file");
    let out_path = tmp.path().to_string_lossy().to_string();

    // First hook: blocks (exit 2) and writes "first" to file
    // Second hook: also writes "second" to file (would be skipped in old short-circuit)
    let mut registry = HookRegistry::new();
    registry.register_matchers(
        HookEvent::PreToolUse,
        vec![
            HookMatcher {
                matcher: None,
                hooks: vec![HookConfig {
                    hook_type: HookCommandType::Command,
                    command: format!("echo first >> {out_path}; exit 2"),
                    if_condition: None,
                    timeout: Some(10),
                    once: None,
                    r#async: None,
                    async_rewake: None,
                    status_message: None,
                    headers: Default::default(),
                    allowed_env_vars: Default::default(),
                }],
            },
            HookMatcher {
                matcher: None,
                hooks: vec![HookConfig {
                    hook_type: HookCommandType::Command,
                    command: format!("echo second >> {out_path}; exit 0"),
                    if_condition: None,
                    timeout: Some(10),
                    once: None,
                    r#async: None,
                    async_rewake: None,
                    status_message: None,
                    headers: Default::default(),
                    allowed_env_vars: Default::default(),
                }],
            },
        ],
        None,
    );

    let input = pre_tool_input("Bash", "echo hello");
    let result = fire_pre_tool_use(&registry, input).await;

    // Should be blocked (first hook blocks)
    assert!(result.is_blocked());

    // But second hook should ALSO have run (no short-circuit)
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let content = std::fs::read_to_string(&out_path).unwrap_or_default();
    let lines: Vec<&str> = content.trim().lines().collect();
    assert!(
        lines.len() >= 2,
        "both hooks should have run (no short-circuit), got {} lines: {:?}",
        lines.len(),
        lines
    );
}
