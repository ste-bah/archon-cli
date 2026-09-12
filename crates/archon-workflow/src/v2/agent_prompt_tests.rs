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

    // The subject is still deduplication: the same universe reaches the request
    // twice, bare and under `taskUniverse`, and must be carried once.
    assert_eq!(prompt.stable_prefix.matches("TASK-1").count(), 1);
    // ...but an Implementation call now carries it DIGESTED — identity only.
    // The criteria travel in the scoped contract block instead, because left
    // whole one reference decomposition puts over 100KB of other tasks'
    // criteria in front of an agent answerable for one of them. So the detail
    // is absent from both halves, and its absence here is the reduction
    // working rather than the universe going missing.
    assert!(!prompt.stable_prefix.contains("universe-only-detail"));
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
    // Matched WITHOUT the enclosing braces. An Implementation call has its
    // scoped task contract inserted alongside its input, so the input is no
    // longer the whole object — but it is still there, and still compact,
    // which is what this test is for. Asserting the whole object would be
    // asserting that nothing else may ever travel with it.
    assert!(prompt.invocation.contains(r#""nested":{"value":"x"}"#));
    assert!(!prompt.stable_prefix.contains("\n  \"first\""));
    assert!(!prompt.invocation.contains("\n  \"nested\""));
}

#[test]
fn workflow_prompt_keeps_empty_inputs_explicit() {
    let prompt = WorkflowV2AgentAdapter::new().build_prompt_parts(&request());

    assert!(prompt.stable_prefix.contains("## Constraints\n```json\n[]"));
    assert!(prompt.invocation.contains("## Input\n```json\nnull"));
}

#[test]
fn planner_prompt_keeps_index_not_full_acceptance_prose() {
    let mut request = request();
    request.call.id = "author-workflow-script".into();
    request.call.method = WorkflowV2HostMethod::Agent;
    request.call.write_mode = None;
    request.input = serde_json::json!({"task_universe": {
        "schema_version":"workflow-v2-task-universe-v1", "source_roots":["tasks"], "tasks":[{
            "canonical_task_id":"UNIT-1", "source_path":"tasks/unit.md", "dependency_ids":[],
            "title":"Unit", "files_expected_to_change":["src/unit.txt"], "focused_tests":["check-unit"],
            "deliverable_contracts":[], "acceptance_criteria":["HUGE_CRITERION".repeat(1000)]
        }]
    }});
    let prompt = build_prompt_parts(&request);
    let text = format!("{}{}", prompt.stable_prefix, prompt.invocation);
    assert!(!text.contains("HUGE_CRITERION"));
    for field in ["UNIT-1", "tasks/unit.md", "src/unit.txt", "check-unit"] { assert!(text.contains(field)); }
}
