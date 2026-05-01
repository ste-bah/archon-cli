/// TASK-HOOK-025: Phase 2 Integration Tests
///
/// Comprehensive integration tests validating all Phase 2 features together:
/// SC-HOOK-005: HTTP hook end-to-end (mock server, POST received, response parsed)
/// SC-HOOK-006: Prompt hook stdout text injected as additionalContext
/// SC-HOOK-007: TOML loading and parsing
/// SC-HOOK-008: Multi-source: user hook allows, policy hook blocks -> verify blocked
/// SC-HOOK-009: All 27 events can fire through registry
/// SC-HOOK-010: Permission update: policy hook addRules -> rule collected
/// SC-HOOK-011: watchPaths: hook returns paths -> paths collected
/// SC-HOOK-012: Elicitation auto-respond: hook returns action -> action collected
/// SC-HOOK-013: Aggregate merge preserves all Phase 2 fields
/// SC-HOOK-014: Combined Phase 2 features in single hook flow
use archon_core::hooks::{
    AggregatedHookResult, ElicitationAction, HookCommandType, HookConfig, HookEvent, HookMatcher,
    HookOutcome, HookRegistry, HookResult, PermissionUpdate, PermissionUpdateDestination,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_registry(event: HookEvent, cmd: &str) -> HookRegistry {
    make_registry_with_type(event, cmd, HookCommandType::Command)
}

fn make_registry_with_type(
    event: HookEvent,
    cmd: &str,
    hook_type: HookCommandType,
) -> HookRegistry {
    let registry = HookRegistry::new();
    registry.register_matchers(
        event,
        vec![HookMatcher {
            matcher: None,
            hooks: vec![HookConfig {
                hook_type,
                command: cmd.to_string(),
                if_condition: None,
                timeout: Some(10),
                once: None,
                r#async: None,
                async_rewake: None,
                status_message: None,
                headers: Default::default(),
                allowed_env_vars: Default::default(),
                enabled: true,
            }],
        }],
        None,
    );
    registry
}

fn make_multi_hook_registry(event: HookEvent, hooks: Vec<(&str, HookCommandType)>) -> HookRegistry {
    let registry = HookRegistry::new();
    let hook_configs: Vec<HookConfig> = hooks
        .into_iter()
        .map(|(cmd, ht)| HookConfig {
            hook_type: ht,
            command: cmd.to_string(),
            if_condition: None,
            timeout: Some(10),
            once: None,
            r#async: None,
            async_rewake: None,
            status_message: None,
            headers: Default::default(),
            allowed_env_vars: Default::default(),
            enabled: true,
        })
        .collect();
    registry.register_matchers(
        event,
        vec![HookMatcher {
            matcher: None,
            hooks: hook_configs,
        }],
        None,
    );
    registry
}

async fn fire(registry: &HookRegistry, event: HookEvent) -> AggregatedHookResult {
    let cwd = std::env::current_dir().unwrap_or_default();
    let input = serde_json::json!({
        "hook_event": event.to_string(),
        "tool_name": "Bash",
        "tool_input": {"command": "echo test"}
    });
    registry
        .execute_hooks(event, input, &cwd, "test-session")
        .await
}

// ===========================================================================
// SC-HOOK-005: HTTP hook end-to-end
// ===========================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_phase2_http_hook_end_to_end() {
    use axum::Router;
    use axum::routing::post;
    use serde_json::json;
    use tokio::net::TcpListener;

    // Start mock HTTP server
    let app = Router::new().route(
        "/hook",
        post(|| async {
            axum::Json(json!({
                "outcome": "blocking",
                "reason": "http-policy-violation"
            }))
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let url = format!("http://127.0.0.1:{}/hook", addr.port());

    let client = reqwest::Client::new();
    let config = HookConfig {
        hook_type: HookCommandType::Http,
        command: url,
        if_condition: None,
        timeout: Some(5),
        once: None,
        r#async: None,
        async_rewake: None,
        status_message: None,
        headers: Default::default(),
        allowed_env_vars: Default::default(),
        enabled: true,
    };

    let result =
        archon_core::hooks::execute_http_hook(&config, &json!({"event": "test"}), &client).await;
    assert_eq!(result.outcome, HookOutcome::Blocking);
    assert_eq!(result.reason.as_deref(), Some("http-policy-violation"));
}

// ===========================================================================
// SC-HOOK-006: Prompt hook stdout as additionalContext
// ===========================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_phase2_prompt_hook_additional_context() {
    let registry = make_registry_with_type(
        HookEvent::PreToolUse,
        "echo 'Be careful with destructive operations'",
        HookCommandType::Prompt,
    );
    let result = fire(&registry, HookEvent::PreToolUse).await;

    assert!(!result.is_blocked());
    assert_eq!(
        result.additional_contexts,
        vec!["Be careful with destructive operations"],
    );
}

// ===========================================================================
// SC-HOOK-007: TOML loading and parsing
// ===========================================================================

#[test]
fn test_phase2_toml_parsing() {
    let toml_content = r#"
[hooks.PreToolUse]
[[hooks.PreToolUse.matchers]]
hooks = [
    { type = "command", command = "echo checking" }
]
"#;
    let settings = archon_core::hooks::parse_hooks_toml(toml_content).unwrap();
    assert!(settings.contains_key(&HookEvent::PreToolUse));
    let matchers = &settings[&HookEvent::PreToolUse];
    assert_eq!(matchers.len(), 1);
    assert_eq!(matchers[0].hooks.len(), 1);
    assert_eq!(matchers[0].hooks[0].command, "echo checking");
}

// ===========================================================================
// SC-HOOK-008: Multi-source: user allows, policy blocks -> blocked wins
// ===========================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_phase2_multi_source_policy_blocks() {
    // First hook: allows (exit 0)
    // Second hook: blocks (exit 2 with reason)
    let registry = make_multi_hook_registry(
        HookEvent::PreToolUse,
        vec![
            ("exit 0", HookCommandType::Command),
            ("echo 'policy-block' >&2; exit 2", HookCommandType::Command),
        ],
    );
    let result = fire(&registry, HookEvent::PreToolUse).await;

    assert!(result.is_blocked(), "policy block should win");
    let reason = result.block_reason().unwrap();
    assert!(reason.contains("policy-block"), "reason: {reason}");
}

// ===========================================================================
// SC-HOOK-009: All 27 events can register and fire
// ===========================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_phase2_all_27_events_fire() {
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

    assert_eq!(events.len(), 27, "must test all 27 events");

    for event in events {
        let registry = make_registry(event.clone(), "echo ok");
        let result = fire(&registry, event).await;
        assert!(!result.is_blocked(), "echo ok should not block");
    }
}

// ===========================================================================
// SC-HOOK-010: Permission update collected from hook
// ===========================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_phase2_permission_update_collected() {
    // Hook that returns JSON with updated_permissions
    let cmd = r#"printf '{"outcome":"success","updated_permissions":[{"type":"addRules","destination":"session","rules":["allow:Bash(echo *)"]}]}'"#;
    let registry = make_registry(HookEvent::PermissionRequest, cmd);
    let result = fire(&registry, HookEvent::PermissionRequest).await;

    assert!(!result.is_blocked());
    assert_eq!(result.updated_permissions.len(), 1);
    match &result.updated_permissions[0] {
        PermissionUpdate::AddRules { destination, rules } => {
            assert_eq!(*destination, PermissionUpdateDestination::Session);
            assert_eq!(rules, &vec!["allow:Bash(echo *)".to_string()]);
        }
        other => panic!("expected AddRules, got: {other:?}"),
    }
}

// ===========================================================================
// SC-HOOK-011: watchPaths collected from hook
// ===========================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_phase2_watch_paths_collected() {
    let cmd =
        r#"printf '{"outcome":"success","watch_paths":["/tmp/config.toml","/tmp/rules.yaml"]}'"#;
    let registry = make_registry(HookEvent::PostToolUse, cmd);
    let result = fire(&registry, HookEvent::PostToolUse).await;

    assert!(!result.is_blocked());
    assert_eq!(
        result.watch_paths,
        vec!["/tmp/config.toml", "/tmp/rules.yaml"]
    );
}

// ===========================================================================
// SC-HOOK-012: Elicitation auto-respond (action collected)
// ===========================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_phase2_elicitation_auto_respond_accept() {
    let cmd = r#"printf '{"outcome":"success","elicitation_action":"accept","elicitation_content":{"answer":"yes"}}'"#;
    let registry = make_registry(HookEvent::Elicitation, cmd);
    let result = fire(&registry, HookEvent::Elicitation).await;

    assert!(!result.is_blocked());
    assert_eq!(result.elicitation_action, Some(ElicitationAction::Accept));
    assert_eq!(
        result.elicitation_content,
        Some(serde_json::json!({"answer": "yes"}))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_phase2_elicitation_auto_respond_cancel() {
    let cmd = r#"printf '{"outcome":"success","elicitation_action":"cancel"}'"#;
    let registry = make_registry(HookEvent::Elicitation, cmd);
    let result = fire(&registry, HookEvent::Elicitation).await;

    assert_eq!(result.elicitation_action, Some(ElicitationAction::Cancel));
}

// ===========================================================================
// SC-HOOK-013: Aggregate merge preserves all Phase 2 fields
// ===========================================================================

#[test]
fn test_phase2_aggregate_merge_all_fields() {
    let mut agg = AggregatedHookResult::new();

    // Hook 1: permissions + watch_paths
    let result1 = HookResult {
        updated_permissions: vec![PermissionUpdate::AddRules {
            destination: PermissionUpdateDestination::Session,
            rules: vec!["allow:Read(*)".to_string()],
        }],
        watch_paths: vec!["/tmp/a.txt".to_string()],
        ..HookResult::allow()
    };
    agg.merge(result1);

    // Hook 2: elicitation + more watch_paths + system_message
    let result2 = HookResult {
        elicitation_action: Some(ElicitationAction::Accept),
        elicitation_content: Some(serde_json::json!({"v": 1})),
        watch_paths: vec!["/tmp/b.txt".to_string()],
        system_message: Some("heads up".to_string()),
        ..HookResult::allow()
    };
    agg.merge(result2);

    // Verify all fields preserved
    assert_eq!(agg.updated_permissions.len(), 1, "permissions collected");
    assert_eq!(
        agg.watch_paths,
        vec!["/tmp/a.txt", "/tmp/b.txt"],
        "watch_paths collected from both"
    );
    assert_eq!(
        agg.elicitation_action,
        Some(ElicitationAction::Accept),
        "elicitation action"
    );
    assert_eq!(
        agg.elicitation_content,
        Some(serde_json::json!({"v": 1})),
        "elicitation content"
    );
    assert_eq!(agg.system_messages, vec!["heads up"], "system messages");
}

// ===========================================================================
// SC-HOOK-014: Combined Phase 2 features in single hook flow
// ===========================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_phase2_combined_features_single_flow() {
    // A single command hook that returns all Phase 2 fields at once
    let cmd = r#"printf '{"outcome":"success","system_message":"combined-test","updated_permissions":[{"type":"addRules","destination":"session","rules":["allow:Glob(*)"]}],"watch_paths":["/tmp/combined.txt"],"elicitation_action":"decline","elicitation_content":{"reason":"automated"}}'"#;
    let registry = make_registry(HookEvent::Elicitation, cmd);
    let result = fire(&registry, HookEvent::Elicitation).await;

    assert!(!result.is_blocked());
    assert_eq!(result.system_messages, vec!["combined-test"]);
    assert_eq!(result.updated_permissions.len(), 1);
    assert_eq!(result.watch_paths, vec!["/tmp/combined.txt"]);
    assert_eq!(result.elicitation_action, Some(ElicitationAction::Decline));
    assert_eq!(
        result.elicitation_content,
        Some(serde_json::json!({"reason": "automated"}))
    );
}

// ===========================================================================
// Additional: Prompt + Command hooks on same event, both produce results
// ===========================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_phase2_mixed_hook_types_same_event() {
    let registry = make_multi_hook_registry(
        HookEvent::PreToolUse,
        vec![
            // Prompt hook: adds context
            ("echo 'context-from-prompt'", HookCommandType::Prompt),
            // Command hook: adds system message via JSON
            (
                r#"printf '{"outcome":"success","system_message":"from-command"}'"#,
                HookCommandType::Command,
            ),
        ],
    );
    let result = fire(&registry, HookEvent::PreToolUse).await;

    assert!(!result.is_blocked());
    assert_eq!(result.additional_contexts, vec!["context-from-prompt"]);
    assert_eq!(result.system_messages, vec!["from-command"]);
}

// ===========================================================================
// Additional: Block wins even when mixed with allow hooks
// ===========================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_phase2_block_wins_over_allows() {
    let registry = make_multi_hook_registry(
        HookEvent::PreToolUse,
        vec![
            ("exit 0", HookCommandType::Command),
            (
                r#"printf '{"outcome":"success","system_message":"info"}'"#,
                HookCommandType::Command,
            ),
            (
                "echo 'blocked-reason' >&2; exit 2",
                HookCommandType::Command,
            ),
        ],
    );
    let result = fire(&registry, HookEvent::PreToolUse).await;

    assert!(result.is_blocked());
    assert!(result.block_reason().unwrap().contains("blocked-reason"));
    // System message from the allow hook should still be collected
    assert_eq!(result.system_messages, vec!["info"]);
}
