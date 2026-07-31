use super::*;
use crate::{WorkflowV2HostMethod, WorkflowV2HostOptions};

fn request() -> WorkflowV2AgentRequest {
    WorkflowV2AgentRequest {
        call: WorkflowV2HostCall {
            id: "call-1".to_string(),
            method: WorkflowV2HostMethod::Implementation,
            write_mode: Some(WorkflowV2WriteMode::Coordinated),
            options: WorkflowV2HostOptions::default(),
        },
        role: "coder".to_string(),
        task: "Implement TASK-1".to_string(),
        constraints: Vec::new(),
        input: serde_json::Value::Null,
        repository_root: Some("/repo".to_string()),
        project_artifacts: Default::default(),
        target_files: vec!["src/lib.rs".to_string()],
        target_ownership_scopes: Vec::new(),
    }
}

#[test]
fn workflow_prompt_builder_stays_within_function_size_limit() {
    let source = include_str!("agent_prompt.rs");
    let start = source
        .find("pub(super) fn build_prompt_parts")
        .expect("prompt builder");
    let function = source[start..]
        .split_once("\nfn build_stable_prefix")
        .expect("next function")
        .0;

    assert!(
        function.lines().count() < 50,
        "build_prompt_parts spans {} lines",
        function.lines().count()
    );
}

#[test]
fn workflow_prompt_splits_stable_prefix_from_volatile_call_data() {
    let mut first = request();
    first.input = serde_json::json!({
        "task_universe": {
            "schema_version":"workflow-v2-task-universe-v1",
            "source_roots":["project-tasks"],
            "tasks":[{"canonical_task_id":"TASK-1","description":"stable"}]
        },
        "wave": 1
    });
    let mut second = first.clone();
    second.call.id = "call-2".into();
    second.input["wave"] = serde_json::json!(2);
    let adapter = WorkflowV2AgentAdapter::new();

    let first_prompt = adapter.build_prompt_parts(&first);
    let second_prompt = adapter.build_prompt_parts(&second);

    assert_eq!(first_prompt.stable_prefix, second_prompt.stable_prefix);
    assert_ne!(first_prompt.invocation, second_prompt.invocation);
    assert!(first_prompt.stable_prefix.contains("task_universe"));
    assert!(!first_prompt.stable_prefix.contains("call-1"));
    assert!(first_prompt.invocation.contains("call-1"));
}

#[test]
fn workflow_prompt_extracts_nested_task_universe_aliases_without_duplication() {
    let universe = serde_json::json!({
        "schema_version":"workflow-v2-task-universe-v1",
        "source_roots":["project-tasks"],
        "tasks":[{"canonical_task_id":"TASK-1","description":"universe-only-detail"}]
    });
    let mut request = request();
    request.input = serde_json::json!([
        universe,
        {"taskUniverse": universe, "wave": 2}
    ]);

    let prompt = WorkflowV2AgentAdapter::new().build_prompt_parts(&request);

    assert_eq!(
        prompt.stable_prefix.matches("universe-only-detail").count(),
        1
    );
    assert!(!prompt.invocation.contains("universe-only-detail"));
    assert!(prompt.invocation.contains(r#"{"wave":2}"#));
}

#[test]
fn workflow_prompt_does_not_extract_unverified_task_shaped_payloads() {
    let mut request = request();
    request.input = serde_json::json!({
        "candidate": {"tasks":[{"id":"TASK-1"}]},
        "wave": 3
    });

    let prompt = WorkflowV2AgentAdapter::new().build_prompt_parts(&request);

    assert!(prompt.stable_prefix.contains("null"));
    assert!(prompt.invocation.contains("TASK-1"));
}

#[test]
fn workflow_prompt_uses_compact_json_for_input_and_constraints() {
    let mut request = request();
    request.constraints = vec!["first".into(), "second".into()];
    request.input = serde_json::json!({"nested":{"value":"x"}});

    let prompt = WorkflowV2AgentAdapter::new().build_prompt_parts(&request);

    assert!(prompt.stable_prefix.contains(r#"["first","second"]"#));
    assert!(prompt.invocation.contains(r#"{"nested":{"value":"x"}}"#));
    assert!(!prompt.stable_prefix.contains("\n  \"first\""));
    assert!(!prompt.invocation.contains("\n  \"nested\""));
}

#[test]
fn workflow_prompt_keeps_empty_inputs_explicit() {
    let prompt = WorkflowV2AgentAdapter::new().build_prompt_parts(&request());

    assert!(prompt.stable_prefix.contains("## Constraints\n```json\n[]"));
    assert!(prompt.invocation.contains("## Input\n```json\nnull"));
}
