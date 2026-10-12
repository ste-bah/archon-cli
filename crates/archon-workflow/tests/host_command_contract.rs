use std::collections::BTreeMap;

use archon_workflow::v2::script::{
    ScriptHostRequest, dry_run_workflow_plan, parse_host_command_request,
};
use archon_workflow::{
    HostCommandRequest, HostCommandSubject, WorkflowV2HostMethod, host_command_call_id,
};

#[test]
fn host_command_method_parse_and_as_str_round_trip() {
    let method = WorkflowV2HostMethod::parse("hostCommand").expect("hostCommand method");
    assert_eq!(method, WorkflowV2HostMethod::HostCommand);
    assert_eq!(method.as_str(), "hostCommand");
    assert_eq!(serde_json::to_string(&method).unwrap(), "\"hostCommand\"");
}

#[test]
fn host_command_rejects_empty_command_id() {
    let request = ScriptHostRequest {
        id: "hostCommand#1".into(),
        options: serde_json::json!({"commandId": "  ", "stdin": null}),
        source: None,
    };

    let error = parse_host_command_request(&request).expect_err("empty command id must fail");
    assert!(
        error
            .to_string()
            .contains("non-empty command capability id")
    );
}

#[test]
fn host_command_accepts_only_bounded_stdin() {
    let request = HostCommandRequest::new("task-set-lint", Some("candidate".into())).unwrap();
    assert_eq!(request.command_id, "task-set-lint");
    assert_eq!(request.stdin.as_deref(), Some("candidate"));
    assert_eq!(request.target, None);

    let oversized = "x".repeat(HostCommandRequest::MAX_STDIN_BYTES + 1);
    let error = HostCommandRequest::new("task-set-lint", Some(oversized))
        .expect_err("oversized stdin must fail");
    assert!(error.to_string().contains("stdin exceeds"));
}

#[test]
fn old_host_command_records_without_target_deserialize_for_replay() {
    let old_record = serde_json::json!({"commandId":"land-task-body","stdin":"old body"});
    let request: HostCommandRequest = serde_json::from_value(old_record).unwrap();
    assert_eq!(request.target, None);
    assert_eq!(request.stdin.as_deref(), Some("old body"));
}

#[test]
fn host_command_target_rejects_missing_empty_and_extra_fields() {
    for target in [
        serde_json::json!({"taskId":"TASK-X-010"}),
        serde_json::json!({"taskId":"", "fileName":"TASK-X-010.md"}),
        serde_json::json!({"taskId":"TASK-X-010", "fileName":"TASK-X-010.md", "extra":true}),
    ] {
        let request = ScriptHostRequest {
            id: "hostCommand#1".into(),
            options: serde_json::json!({"commandId":"land-task-body", "stdin":"body", "target":target}),
            source: None,
        };
        assert!(parse_host_command_request(&request).is_err(), "{target}");
    }
}

#[test]
fn host_command_rejects_script_authored_process_fields() {
    for field in [
        "executable",
        "argv",
        "cwd",
        "environment",
        "timeout",
        "limits",
        "destination",
        "writeSet",
        "reuseKey",
    ] {
        let request = ScriptHostRequest {
            id: "hostCommand#1".into(),
            options: serde_json::json!({
                "commandId": "task-set-lint",
                "stdin": null,
                field: "model-authored"
            }),
            source: None,
        };
        let error =
            parse_host_command_request(&request).expect_err("process authority must be rejected");
        assert!(error.to_string().contains(field), "{field}: {error}");
    }
}

#[test]
fn host_command_call_identity_is_domain_separated_and_length_framed() {
    let mut left_tokens = BTreeMap::new();
    left_tokens.insert("a".to_string(), "bc".to_string());
    let mut right_tokens = BTreeMap::new();
    right_tokens.insert("ab".to_string(), "c".to_string());

    let left = host_command_call_id("x", "yz", "r", &left_tokens, b"payload");
    let right = host_command_call_id("xy", "z", "r", &right_tokens, b"payload");
    assert_ne!(left, right, "ambiguous raw concatenations must not collide");

    assert_eq!(
        left,
        host_command_call_id("x", "yz", "r", &left_tokens, b"payload"),
        "identity must be stable",
    );
    assert_ne!(
        left,
        host_command_call_id("x", "yz", "r", &left_tokens, b"payload2"),
        "exact stdin bytes are identity-bearing",
    );
}

#[test]
fn host_command_target_changes_identity_while_legacy_identity_stays_stable() {
    let tokens = BTreeMap::new();
    let legacy = host_command_call_id("land-task-body", "catalog", "rev", &tokens, b"body");
    let target = HostCommandSubject {
        task_id: "TASK-X-010".into(),
        file_name: "TASK-X-010.md".into(),
    };
    let mut targeted_tokens = tokens.clone();
    targeted_tokens.insert("HOST_COMMAND_TARGET_TASK_ID".into(), target.task_id.clone());
    targeted_tokens.insert(
        "HOST_COMMAND_TARGET_FILE_NAME".into(),
        target.file_name.clone(),
    );
    let targeted = host_command_call_id(
        "land-task-body",
        "catalog",
        "rev",
        &targeted_tokens,
        b"body",
    );
    assert_ne!(legacy, targeted);
    assert_eq!(
        legacy,
        host_command_call_id("land-task-body", "catalog", "rev", &tokens, b"body")
    );
}

#[tokio::test]
async fn raw_w_host_command_reaches_authoritative_host_bridge_without_prelude() {
    let script = r#"
async function workflow(w) {
  const result = await w.hostCommand("task-set-lint", {
    stdin: null,
    target: { taskId: "TASK-X-010", fileName: "TASK-X-010.md" }
  });
  const required = [
    "exitCode", "stdout", "stderr", "stdoutBytes", "stderrBytes",
    "timedOut", "interrupted", "stdoutTruncated", "stderrTruncated",
    "gateEnvelope", "publicationReceipt", "dryRun"
  ];
  for (const key of required) {
    if (!(key in result)) throw new Error(`missing ${key}`);
  }
  return result;
}
"#;

    let calls = dry_run_workflow_plan(script, None).await.unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].method, WorkflowV2HostMethod::HostCommand);
    let request = calls[0]
        .options
        .host_command
        .as_ref()
        .expect("typed host-command request");
    assert_eq!(request.command_id, "task-set-lint");
    assert_eq!(request.stdin, None);
    assert_eq!(
        request
            .target
            .as_ref()
            .map(|target| target.task_id.as_str()),
        Some("TASK-X-010")
    );
}

#[tokio::test]
async fn raw_w_host_command_rejects_process_authority_before_dry_run_recording() {
    let script = r#"
async function workflow(w) {
  return await w.hostCommand("task-set-lint", {
    stdin: null,
    executable: "/bin/sh"
  });
}
"#;

    let error = dry_run_workflow_plan(script, None)
        .await
        .expect_err("raw process override must fail");
    assert!(error.to_string().contains("executable"), "{error}");
}
