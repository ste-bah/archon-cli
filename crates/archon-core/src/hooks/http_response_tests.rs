use super::*;
use crate::hooks::{ElicitationAction, PermissionBehavior};

async fn status_result(status: u16, body: &str, policy: &str) -> HookResult {
    let config: HookConfig = serde_json::from_value(serde_json::json!({
        "type":"http", "command":"https://example.invalid/hook", "on_failure":policy
    }))
    .unwrap();
    let response: reqwest::Response = axum::http::Response::builder()
        .status(status)
        .body(body.to_owned())
        .unwrap()
        .into();
    read_hook_response(
        &config,
        "PreToolUse",
        response,
        &NoProgressWindow::new(Duration::from_secs(1)),
        "https://example.invalid",
    )
    .await
    .result
}

async fn rejected_status(status: u16, body: &str) {
    let result = status_result(status, body, "allow").await;
    assert_eq!(result.outcome, HookOutcome::NonBlockingError, "{result:?}");
    assert!(result.reason.unwrap().contains(&status.to_string()));
    assert_ne!(result.permission_behavior, Some(PermissionBehavior::Allow));
}
#[tokio::test]
async fn authentication_rejection_is_not_success() {
    rejected_status(401, "unauthorized").await;
}
#[tokio::test]
async fn authorization_rejection_is_not_success() {
    rejected_status(403, "forbidden").await;
}
#[tokio::test]
async fn server_error_cannot_return_a_success_body() {
    rejected_status(500, r#"{"outcome":"success"}"#).await;
}
#[tokio::test]
async fn error_status_cannot_grant_permission() {
    rejected_status(
        403,
        r#"{"outcome":"success","permission_behavior":"allow"}"#,
    )
    .await;
}

// An explicit refusal in an error response is a decision, not a transport
// failure: an allowing failure policy must never downgrade it.
#[tokio::test]
async fn forbidden_blocking_body_blocks_under_allow_policy() {
    let body = r#"{"outcome":"blocking","reason":"rm -rf denied"}"#;
    let result = status_result(403, body, "allow").await;
    assert_eq!(result.outcome, HookOutcome::Blocking, "{result:?}");
    assert_eq!(result.reason.as_deref(), Some("rm -rf denied"));
}
#[tokio::test]
async fn client_error_deny_body_is_kept_under_allow_policy() {
    let body =
        r#"{"outcome":"success","permission_behavior":"deny","permission_decision_reason":"no"}"#;
    let result = status_result(401, body, "allow").await;
    assert_eq!(result.permission_behavior, Some(PermissionBehavior::Deny));
    assert_eq!(result.permission_decision_reason.as_deref(), Some("no"));
}
#[tokio::test]
async fn server_error_blocking_body_keeps_its_reason_under_block_policy() {
    let body = r#"{"outcome":"blocking","reason":"quota policy refused"}"#;
    let result = status_result(503, body, "block").await;
    assert_eq!(result.outcome, HookOutcome::Blocking, "{result:?}");
    assert_eq!(result.reason.as_deref(), Some("quota policy refused"));
}
#[tokio::test]
async fn error_status_without_a_decision_follows_block_policy() {
    let result = status_result(500, "not json", "block").await;
    assert_eq!(result.outcome, HookOutcome::Blocking, "{result:?}");
    assert!(result.reason.unwrap().contains("500"));
}

// Every refusal signal in an error body is kept under every policy, and the
// error status is still recorded: a stop or a decline is never lost.
#[tokio::test]
async fn forbidden_stop_body_ends_the_turn_under_allow_policy() {
    let body = r#"{"outcome":"success","prevent_continuation":true,"stop_reason":"quota"}"#;
    let result = status_result(403, body, "allow").await;
    assert_eq!(result.prevent_continuation, Some(true), "{result:?}");
    assert_eq!(result.stop_reason.as_deref(), Some("quota"));
    assert_eq!(result.outcome, HookOutcome::NonBlockingError);
    assert!(result.reason.unwrap().contains("403"));
}
#[tokio::test]
async fn server_error_stop_body_is_kept_under_block_policy() {
    let body = r#"{"outcome":"success","prevent_continuation":true,"stop_reason":"halt"}"#;
    let result = status_result(500, body, "block").await;
    assert_eq!(result.outcome, HookOutcome::Blocking, "{result:?}");
    assert_eq!(result.prevent_continuation, Some(true));
    assert_eq!(result.stop_reason.as_deref(), Some("halt"));
}
#[tokio::test]
async fn forbidden_elicitation_decline_is_kept_under_allow_policy() {
    let body = r#"{"outcome":"success","elicitation_action":"decline"}"#;
    let result = status_result(403, body, "allow").await;
    assert_eq!(
        result.elicitation_action,
        Some(ElicitationAction::Decline),
        "{result:?}"
    );
    assert_eq!(result.outcome, HookOutcome::NonBlockingError);
}
#[tokio::test]
async fn throttled_elicitation_cancel_is_kept_under_block_policy() {
    let body = r#"{"outcome":"success","elicitation_action":"cancel"}"#;
    let result = status_result(429, body, "block").await;
    assert_eq!(result.elicitation_action, Some(ElicitationAction::Cancel));
    assert_eq!(result.outcome, HookOutcome::Blocking, "{result:?}");
}
#[tokio::test]
async fn error_body_keeps_every_refusal_together() {
    let body = r#"{"outcome":"blocking","reason":"no","permission_behavior":"deny",
        "prevent_continuation":true,"stop_reason":"stop","elicitation_action":"decline"}"#;
    let result = status_result(502, body, "allow").await;
    assert_eq!(result.outcome, HookOutcome::Blocking, "{result:?}");
    assert_eq!(result.reason.as_deref(), Some("no"));
    assert_eq!(result.permission_behavior, Some(PermissionBehavior::Deny));
    assert_eq!(result.prevent_continuation, Some(true));
    assert_eq!(result.elicitation_action, Some(ElicitationAction::Decline));
}
// A refusal does not carry grants or edits from the same error body through.
#[tokio::test]
async fn error_stop_body_drops_grants_and_edits() {
    let body = r#"{"outcome":"success","prevent_continuation":true,
        "permission_behavior":"allow","source_authority":"policy",
        "updated_input":{"cmd":"rm -rf /"},"additional_context":"x"}"#;
    let result = status_result(403, body, "allow").await;
    assert_eq!(result.prevent_continuation, Some(true), "{result:?}");
    assert_eq!(result.permission_behavior, None);
    assert!(result.updated_input.is_none());
    assert!(result.additional_context.is_none());
}
#[tokio::test]
async fn error_elicitation_accept_and_ask_are_not_kept() {
    let body = r#"{"outcome":"success","elicitation_action":"accept","permission_behavior":"ask",
        "prevent_continuation":false}"#;
    let result = status_result(403, body, "allow").await;
    assert_eq!(result.elicitation_action, None, "{result:?}");
    assert_eq!(result.permission_behavior, None);
    assert_ne!(result.prevent_continuation, Some(true));
    assert_eq!(result.outcome, HookOutcome::NonBlockingError);
}
