use super::{WorkflowEventLog, pause_cause, sanitize_value};
use crate::{WorkflowEventKind, WorkflowStore};
use serde_json::json;

#[test]
fn prompt_text_redacts_credentials_without_corrupting_prose_or_shell_variables() {
    let prose =
        "Secrets, API keys, authorization values, cookies, and session IDs must never be logged";
    let loop_script = r#"for token in alpha beta; do grep -qF -- "$token"; done"#;
    let ordinary = "This secret explains the behavior and uses a token as a variable.";
    let ordinary_assignment = "author=value is a normal assignment.";
    for text in [prose, loop_script, ordinary, ordinary_assignment] {
        assert_eq!(sanitize_value(json!(text)).as_str(), Some(text));
    }

    let credentials = [
        concat!("API_KEY=sk", "_test_123456789012345678901234"),
        concat!("Bearer gh", "p_123456789012345678901234567890123456"),
        "password=hunter2password",
        concat!("sk", "-proj-12345678901234567890"),
    ];
    let expected_secrets = [
        concat!("sk", "_test_123456789012345678901234"),
        concat!("gh", "p_123456789012345678901234567890123456"),
        "hunter2password",
        concat!("sk", "-proj-12345678901234567890"),
    ];
    for (text, secret) in credentials.into_iter().zip(expected_secrets) {
        let clean = sanitize_value(json!(text));
        let clean = clean.as_str().unwrap();
        assert!(!clean.contains(secret), "leaked {secret}: {clean}");
        assert!(clean.contains("<redacted>"), "missing marker: {clean}");
    }
}

#[test]
fn pause_cause_is_mandatory_and_uses_recorded_evidence_when_available() {
    let detail = pause_cause(
        json!({"action":"pause","pause_id":"call-7","reason":"stalled"}),
        "run-1",
    );
    assert_eq!(detail["cause_kind"], "pause");
    assert_eq!(detail["cause_reason"], "stalled");
    assert_eq!(detail["call_id"], "call-7");

    let detail = pause_cause(json!("legacy detail"), "run-2");
    assert_eq!(detail["detail"], "legacy detail");
    assert_eq!(detail["cause_kind"], "workflow_pause");
    assert_eq!(detail["cause_reason"], "legacy detail");
    assert_eq!(detail["call_id"], "run-control:run-2");

    let detail = pause_cause(json!({"evidence":"stdout cap=256, observed=300"}), "run-3");
    assert_eq!(detail["cause_reason"], "stdout cap=256, observed=300");

    let detail = pause_cause(
        json!({"event":"script_pause", "pause_id":"pause-7", "evidence":{"summary":"host judge needs review", "kind":"gate"}}),
        "run-3",
    );
    assert_eq!(detail["cause_reason"], "host judge needs review");
    assert_eq!(detail["call_id"], "pause-7");

    let detail = pause_cause(
        json!({
            "cause_kind": null, "event": "host_command_pause",
            "cause_reason": "", "evidence": "stderr output cap exceeded",
            "call_id": null, "pause_id": "call-8"
        }),
        "run-4",
    );
    assert_eq!(detail["cause_kind"], "host_command_pause");
    assert_eq!(detail["cause_reason"], "stderr output cap exceeded");
    assert_eq!(detail["call_id"], "call-8");
}

#[test]
fn emitted_script_pause_event_sanitizes_nested_evidence_before_stringifying_it() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let run = store
        .create_run(crate::WorkflowSpec {
            schema: crate::spec::WORKFLOW_SCHEMA.into(),
            name: "pause event".into(),
            task: "test".into(),
            target_repository_root: None,
            max_parallelism: 1,
            max_agents: 1,
            stages: Vec::new(),
            permissions: Default::default(),
            learning_hooks: Vec::new(),
        })
        .unwrap();
    let event = WorkflowEventLog::new(store)
        .emit(
            &run.id,
            1,
            WorkflowEventKind::Paused,
            json!({
                "event":"script_pause",
                "pause_id":"pause-acceptance-2",
                "evidence":{
                    "kind":"host_command",
                    "context":{
                        "api_key":"api-secret",
                        "nested":{
                            "token":"token-secret",
                            "authorization":"Bearer auth-secret",
                            "password":"password-secret"
                        }
                    }
                }
            }),
        )
        .unwrap();
    assert_eq!(event.detail["cause_kind"], "script_pause");
    let reason = event.detail["cause_reason"].as_str().unwrap();
    assert!(reason.contains("\"kind\":\"host_command\""), "{reason}");
    for secret in [
        "api-secret",
        "token-secret",
        "auth-secret",
        "password-secret",
    ] {
        assert!(!reason.contains(secret), "leaked {secret}: {reason}");
    }
    assert_eq!(event.detail["call_id"], "pause-acceptance-2");
}

#[test]
fn nested_secret_field_values_are_redacted_before_evidence_becomes_cause_reason() {
    let evidence = json!({
        "category": "host_command",
        "context": {
            "api_key": "api-secret",
            "nested": {
                "token": "token-secret",
                "authorization": "Bearer auth-secret",
                "password": "password-secret"
            }
        }
    });
    let sanitized = sanitize_value(json!({"evidence": evidence}));
    let detail = pause_cause(sanitized.clone(), "run-secret-test");
    let reason = detail["cause_reason"].as_str().unwrap();
    assert!(reason.contains("\"category\":\"host_command\""), "{reason}");
    for secret in [
        "api-secret",
        "token-secret",
        "auth-secret",
        "password-secret",
    ] {
        assert!(!reason.contains(secret), "leaked {secret}: {reason}");
    }
    assert!(sanitized["evidence"]["context"].get("api_key").is_none());
    assert!(
        sanitized["evidence"]["context"]["nested"]
            .get("token")
            .is_none()
    );
    assert!(
        sanitized["evidence"]["context"]["nested"]
            .get("authorization")
            .is_none()
    );
    assert!(
        sanitized["evidence"]["context"]["nested"]
            .get("password")
            .is_none()
    );
}

#[test]
fn baseline_forbidden_field_substrings_are_removed_and_prose_survives() {
    let sanitized = sanitize_value(json!({
        "raw_text": "private",
        "provider_thinking": "private",
        "model_reasoning": "private",
        "reasoning_encrypted": "private",
        "encrypted_reasoning": "private",
        "oauth_token": "private",
        "access_token": "private",
        "refresh_token": "private",
        "provider_api_key": "private",
        "authorization_header": "private",
        "password_hint": "private",
        "message": "token budget; password reset flow; the secret is nearby; $HOME $PATH"
    }));
    for key in [
        "raw_text",
        "provider_thinking",
        "model_reasoning",
        "reasoning_encrypted",
        "encrypted_reasoning",
        "oauth_token",
        "access_token",
        "refresh_token",
        "provider_api_key",
        "authorization_header",
        "password_hint",
    ] {
        assert!(
            sanitized.get(key).is_none(),
            "field {key} survived: {sanitized}"
        );
    }
    assert_eq!(
        sanitized["message"],
        "token budget; password reset flow; the secret is nearby; $HOME $PATH"
    );
}
