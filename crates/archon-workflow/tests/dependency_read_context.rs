use archon_workflow::*;
use serde_json::json;
#[test]
fn transitive_consumed_artifacts_reach_consumer_without_write_ownership() {
    let universe:task_universe::WorkflowV2TaskUniverse=serde_json::from_value(json!({
        "schema_version":"task-universe-v1","source_roots":[],"tasks":[
          {"canonical_task_id":"TASK-001","source_path":"/project/tasks/a.md","deliverable_contracts":[{"kind":"report","artifact_path":"reports/audit.md"}]},
          {"canonical_task_id":"TASK-002","source_path":"/project/tasks/b.md","dependency_ids":["TASK-001"],"dependencies":[{"task_id":"TASK-001","consumes":[{"artifact_path":"reports/audit.md"}],"ordering_only":false}]},
          {"canonical_task_id":"TASK-003","source_path":"/project/tasks/c.md","dependency_ids":["TASK-002"]}
        ]})).unwrap();
    let call = WorkflowV2HostCall {
        id: "write-consumer".into(),
        method: WorkflowV2HostMethod::Implementation,
        write_mode: Some(WorkflowV2WriteMode::Worktree),
        options: WorkflowV2HostOptions {
            target_files: vec!["src/consumer.rs".into()],
            ..Default::default()
        },
    };
    let e = WorkflowV2CallExecution {
        call,
        input: json!({"item":{"canonical_task_ids":["TASK-003"],"target_files":["src/consumer.rs"]},"_workflow_project_artifact_policy":{"project_root":"/project"}}),
        depends_on: vec![],
    };
    let req =
        v2::call_data::v2_agent_request("implement", Some("/repo".into()), &e, Some(&universe));
    let prompt = WorkflowV2AgentAdapter::new().build_prompt(&req);
    assert!(
        prompt.contains("reports/audit.md"),
        "transitively consumed report absent from rendered prompt"
    );
    assert!(prompt.contains("read-only dependency context"));
    assert_eq!(req.target_files, vec!["src/consumer.rs"]);
    assert_eq!(
        req.input["task_universe"]["tasks"]
            .as_array()
            .unwrap()
            .len(),
        1,
        "upstream contracts must not become consumer obligations"
    );
    assert!(!req.input["dependency_read_context"].is_null());
}
