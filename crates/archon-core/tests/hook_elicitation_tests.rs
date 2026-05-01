/// TASK-HOOK-024: Elicitation-Specific Hook Outputs tests
///
/// Tests cover:
/// - ElicitationAction enum serialization/deserialization
/// - HookResult with elicitation fields populated
/// - AggregatedHookResult merge preserves elicitation fields (last writer wins)
/// - Command hook returning elicitation action via JSON stdout
/// - No elicitation action → fields remain None
/// - Elicitation hook auto-respond: action=accept bypasses prompt
/// - Elicitation hook auto-respond: action=decline bypasses prompt
/// - Elicitation content passed through
use archon_core::hooks::{
    ElicitationAction, HookCommandType, HookConfig, HookEvent, HookMatcher, HookRegistry,
    HookResult,
};

// ---------------------------------------------------------------------------
// Unit tests: ElicitationAction enum
// ---------------------------------------------------------------------------

#[test]
fn test_elicitation_action_serialize_accept() {
    let action = ElicitationAction::Accept;
    let json = serde_json::to_string(&action).unwrap();
    assert_eq!(json, r#""accept""#);
}

#[test]
fn test_elicitation_action_serialize_decline() {
    let action = ElicitationAction::Decline;
    let json = serde_json::to_string(&action).unwrap();
    assert_eq!(json, r#""decline""#);
}

#[test]
fn test_elicitation_action_serialize_cancel() {
    let action = ElicitationAction::Cancel;
    let json = serde_json::to_string(&action).unwrap();
    assert_eq!(json, r#""cancel""#);
}

#[test]
fn test_elicitation_action_deserialize() {
    let action: ElicitationAction = serde_json::from_str(r#""accept""#).unwrap();
    assert_eq!(action, ElicitationAction::Accept);

    let action: ElicitationAction = serde_json::from_str(r#""decline""#).unwrap();
    assert_eq!(action, ElicitationAction::Decline);

    let action: ElicitationAction = serde_json::from_str(r#""cancel""#).unwrap();
    assert_eq!(action, ElicitationAction::Cancel);
}

// ---------------------------------------------------------------------------
// Unit tests: HookResult with elicitation fields
// ---------------------------------------------------------------------------

#[test]
fn test_hook_result_default_no_elicitation() {
    let result = HookResult::allow();
    assert!(result.elicitation_action.is_none());
    assert!(result.elicitation_content.is_none());
}

#[test]
fn test_hook_result_with_elicitation_fields() {
    let json = r#"{
        "outcome": "success",
        "elicitation_action": "accept",
        "elicitation_content": {"value": "yes please"}
    }"#;
    let result: HookResult = serde_json::from_str(json).unwrap();
    assert_eq!(result.elicitation_action, Some(ElicitationAction::Accept));
    assert_eq!(
        result.elicitation_content,
        Some(serde_json::json!({"value": "yes please"}))
    );
}

#[test]
fn test_hook_result_elicitation_fields_skip_when_none() {
    let result = HookResult::allow();
    let json = serde_json::to_string(&result).unwrap();
    // elicitation fields should not appear when None
    assert!(!json.contains("elicitation_action"));
    assert!(!json.contains("elicitation_content"));
}

// ---------------------------------------------------------------------------
// Unit tests: AggregatedHookResult merge with elicitation fields
// ---------------------------------------------------------------------------

#[test]
fn test_aggregated_merge_elicitation_last_writer_wins() {
    use archon_core::hooks::AggregatedHookResult;

    let mut agg = AggregatedHookResult::new();

    // First hook: accept
    let result1 = HookResult {
        elicitation_action: Some(ElicitationAction::Accept),
        elicitation_content: Some(serde_json::json!({"answer": "first"})),
        ..HookResult::allow()
    };
    agg.merge(result1);
    assert_eq!(agg.elicitation_action, Some(ElicitationAction::Accept));

    // Second hook: decline overwrites
    let result2 = HookResult {
        elicitation_action: Some(ElicitationAction::Decline),
        elicitation_content: Some(serde_json::json!({"answer": "second"})),
        ..HookResult::allow()
    };
    agg.merge(result2);
    assert_eq!(agg.elicitation_action, Some(ElicitationAction::Decline));
    assert_eq!(
        agg.elicitation_content,
        Some(serde_json::json!({"answer": "second"}))
    );
}

#[test]
fn test_aggregated_merge_no_elicitation_does_not_overwrite() {
    use archon_core::hooks::AggregatedHookResult;

    let mut agg = AggregatedHookResult::new();

    // First hook: sets action
    let result1 = HookResult {
        elicitation_action: Some(ElicitationAction::Accept),
        ..HookResult::allow()
    };
    agg.merge(result1);

    // Second hook: no elicitation fields (does not overwrite)
    let result2 = HookResult::allow();
    agg.merge(result2);

    assert_eq!(
        agg.elicitation_action,
        Some(ElicitationAction::Accept),
        "None should not overwrite existing elicitation_action"
    );
}

// ---------------------------------------------------------------------------
// Integration tests: Command hook returning elicitation action via JSON stdout
// ---------------------------------------------------------------------------

fn make_elicitation_registry(event: HookEvent, cmd: &str) -> HookRegistry {
    let mut registry = HookRegistry::new();
    registry.register_matchers(
        event,
        vec![HookMatcher {
            matcher: None,
            hooks: vec![HookConfig {
                hook_type: HookCommandType::Command,
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

async fn fire_elicitation_hook(
    registry: &HookRegistry,
    event: HookEvent,
) -> archon_core::hooks::AggregatedHookResult {
    let cwd = std::env::current_dir().unwrap_or_default();
    let input = serde_json::json!({
        "hook_event": event.to_string(),
        "question": "Do you want to proceed?"
    });
    registry
        .execute_hooks(event, input, &cwd, "test-session")
        .await
}

/// Command hook emitting JSON with elicitation_action=accept → parsed into HookResult.
#[tokio::test(flavor = "multi_thread")]
async fn test_command_hook_elicitation_accept() {
    let cmd = r#"printf '{"outcome":"success","elicitation_action":"accept","elicitation_content":{"value":"auto-yes"}}'"#;
    let registry = make_elicitation_registry(HookEvent::Elicitation, cmd);
    let result = fire_elicitation_hook(&registry, HookEvent::Elicitation).await;

    assert!(!result.is_blocked());
    assert_eq!(result.elicitation_action, Some(ElicitationAction::Accept));
    assert_eq!(
        result.elicitation_content,
        Some(serde_json::json!({"value": "auto-yes"}))
    );
}

/// Command hook emitting JSON with elicitation_action=decline.
#[tokio::test(flavor = "multi_thread")]
async fn test_command_hook_elicitation_decline() {
    let cmd = r#"printf '{"outcome":"success","elicitation_action":"decline"}'"#;
    let registry = make_elicitation_registry(HookEvent::Elicitation, cmd);
    let result = fire_elicitation_hook(&registry, HookEvent::Elicitation).await;

    assert!(!result.is_blocked());
    assert_eq!(result.elicitation_action, Some(ElicitationAction::Decline));
    assert!(result.elicitation_content.is_none());
}

/// Command hook emitting JSON with elicitation_action=cancel.
#[tokio::test(flavor = "multi_thread")]
async fn test_command_hook_elicitation_cancel() {
    let cmd = r#"printf '{"outcome":"success","elicitation_action":"cancel"}'"#;
    let registry = make_elicitation_registry(HookEvent::Elicitation, cmd);
    let result = fire_elicitation_hook(&registry, HookEvent::Elicitation).await;

    assert!(!result.is_blocked());
    assert_eq!(result.elicitation_action, Some(ElicitationAction::Cancel));
}

/// Command hook with no elicitation fields → both remain None.
#[tokio::test(flavor = "multi_thread")]
async fn test_command_hook_no_elicitation_fields() {
    let cmd = r#"printf '{"outcome":"success"}'"#;
    let registry = make_elicitation_registry(HookEvent::Elicitation, cmd);
    let result = fire_elicitation_hook(&registry, HookEvent::Elicitation).await;

    assert!(!result.is_blocked());
    assert!(result.elicitation_action.is_none());
    assert!(result.elicitation_content.is_none());
}
