use super::*;

fn task_universe() -> WorkflowV2TaskUniverse {
    WorkflowV2TaskUniverse {
        schema_version: "workflow-v2-task-universe-v1".to_string(),
        source_roots: vec!["/tmp/tasks".to_string()],
        tasks: vec![
            super::super::workflow_live_task_universe::WorkflowV2TaskUniverseTask {
                canonical_task_id: "TASK-TDL-001".to_string(),
                aliases: vec!["T001".to_string()],
                source_path: "/tmp/TASK-TDL-001.md".to_string(),
                dependency_ids: Vec::new(),
                title: None,
                artifact_requirements: Vec::new(),
            },
            super::super::workflow_live_task_universe::WorkflowV2TaskUniverseTask {
                canonical_task_id: "TASK-TDL-010".to_string(),
                aliases: vec!["T010".to_string()],
                source_path: "/tmp/TASK-TDL-010.md".to_string(),
                dependency_ids: vec!["TASK-TDL-001".to_string()],
                title: None,
                artifact_requirements: Vec::new(),
            },
        ],
    }
}

fn tdl_task_universe() -> WorkflowV2TaskUniverse {
    let task = |canonical: &str, deps: &[&str]| {
        super::super::workflow_live_task_universe::WorkflowV2TaskUniverseTask {
            canonical_task_id: canonical.to_string(),
            aliases: vec![canonical.replace("TASK-TDL-", "T")],
            source_path: format!("/tmp/tasks/{canonical}.md"),
            dependency_ids: deps.iter().map(|dep| dep.to_string()).collect(),
            title: None,
            artifact_requirements: Vec::new(),
        }
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

include!("workflow_live_generated_contract_tests_a.rs");
include!("workflow_live_generated_contract_tests_b.rs");
include!("workflow_live_generated_contract_tests_c.rs");
include!("workflow_live_generated_contract_tests_d.rs");
include!("workflow_live_generated_contract_tests_e.rs");
include!("workflow_live_generated_contract_tests_f.rs");
