use super::*;

fn task_universe() -> WorkflowV2TaskUniverse {
    WorkflowV2TaskUniverse {
        schema_version: "workflow-v2-task-universe-v1".to_string(),
        source_roots: vec!["/tmp/tasks".to_string()],
        tasks: vec![
            crate::task_universe::WorkflowV2TaskUniverseTask {
                canonical_task_id: "TASK-TDL-001".to_string(),
                aliases: vec!["T001".to_string()],
                source_path: "/tmp/TASK-TDL-001.md".to_string(),
                dependency_ids: Vec::new(),
                title: None,
                artifact_requirements: Vec::new(),
                ..Default::default()
            },
            crate::task_universe::WorkflowV2TaskUniverseTask {
                canonical_task_id: "TASK-TDL-010".to_string(),
                aliases: vec!["T010".to_string()],
                source_path: "/tmp/TASK-TDL-010.md".to_string(),
                dependency_ids: vec!["TASK-TDL-001".to_string()],
                title: None,
                artifact_requirements: Vec::new(),
                ..Default::default()
            },
        ],
    }
}

fn tdl_task_universe() -> WorkflowV2TaskUniverse {
    let task = |canonical: &str, deps: &[&str]| crate::task_universe::WorkflowV2TaskUniverseTask {
        canonical_task_id: canonical.to_string(),
        aliases: vec![canonical.replace("TASK-TDL-", "T")],
        source_path: format!("/tmp/tasks/{canonical}.md"),
        dependency_ids: deps.iter().map(|dep| dep.to_string()).collect(),
        title: None,
        artifact_requirements: Vec::new(),
        ..Default::default()
    };
    WorkflowV2TaskUniverse {
        schema_version: "workflow-v2-task-universe-v1".to_string(),
        source_roots: vec!["/tmp/tasks".to_string()],
        tasks: vec![
            task("TASK-TDL-001", &[]),
            task("TASK-TDL-010", &["TASK-TDL-001"]),
            task("TASK-TDL-020", &["TASK-TDL-010"]),
            task("TASK-TDL-030", &["TASK-TDL-020"]),
            task("TASK-TDL-040", &["TASK-TDL-030"]),
            task("TASK-TDL-050", &["TASK-TDL-030"]),
            task("TASK-TDL-060", &["TASK-TDL-030"]),
            task("TASK-TDL-070", &["TASK-TDL-030"]),
            task(
                "TASK-TDL-080",
                &[
                    "TASK-TDL-040",
                    "TASK-TDL-050",
                    "TASK-TDL-060",
                    "TASK-TDL-070",
                ],
            ),
            task("TASK-TDL-090", &["TASK-TDL-080"]),
            task("TASK-TDL-100", &["TASK-TDL-080", "TASK-TDL-090"]),
            task("TASK-TDL-110", &["TASK-TDL-100"]),
            task("TASK-TDL-120", &["TASK-TDL-110"]),
            task("TASK-TDL-130", &["TASK-TDL-120"]),
            task("TASK-TDL-140", &["TASK-TDL-130"]),
        ],
    }
}

#[test]
fn neutral_prd_capabilities_are_stamped_without_task_id_heuristics() {
    let universe = WorkflowV2TaskUniverse {
        schema_version: "workflow-v2-task-universe-v1".to_string(),
        source_roots: vec!["/tmp/demo-tasks".to_string()],
        tasks: vec![crate::task_universe::WorkflowV2TaskUniverseTask {
            canonical_task_id: "TASK-DEMO-017".to_string(),
            source_path: "/tmp/demo-tasks/TASK-DEMO-017.md".to_string(),
            required_env_keys: vec!["DEMO_API_KEY".to_string()],
            required_tools: vec!["fetch_demo_cells".to_string()],
            deliverable_contracts: vec![crate::task_universe::WorkflowV2DeliverableContract {
                kind: "required_universe_registry".to_string(),
                artifact_path: ".archon/demo/coverage.json".to_string(),
                registry_path: Some(".archon/demo/registry.json".to_string()),
                required_universe: true,
                ..Default::default()
            }],
            ..Default::default()
        }],
    };
    let normalized = normalize_generated_item_value(
        &serde_json::json!({
            "item_id": "implement-demo-deliverable",
            "canonical_task_ids": ["TASK-DEMO-017"],
            "target_files": ["src/demo.rs"],
            "focused_verification": ["cargo test demo"],
            "expected_evidence": ["demo deliverable exists"]
        }),
        Some(&universe),
    );

    assert_eq!(
        normalized.value["required_env_keys"],
        serde_json::json!(["DEMO_API_KEY"])
    );
    assert_eq!(
        normalized.value["required_tools"],
        serde_json::json!(["fetch_demo_cells"])
    );
    assert_eq!(
        normalized.value["deliverable_contracts"][0]["artifact_path"],
        ".archon/demo/coverage.json"
    );
}

#[path = "generated_contract_tests_a.rs"]
mod generated_contract_tests_a;
#[path = "generated_contract_tests_b.rs"]
mod generated_contract_tests_b;
#[path = "generated_contract_tests_c.rs"]
mod generated_contract_tests_c;
#[path = "generated_contract_tests_d.rs"]
mod generated_contract_tests_d;
#[path = "generated_contract_tests_e.rs"]
mod generated_contract_tests_e;
#[path = "generated_contract_tests_f.rs"]
mod generated_contract_tests_f;
#[path = "generated_contract_tests_g.rs"]
mod generated_contract_tests_g;
