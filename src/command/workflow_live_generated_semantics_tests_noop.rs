use super::super::workflow_live_generated_scaffold::decomposed_prd_scaffold;
use super::super::workflow_live_task_universe::{
    WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask,
};

#[test]
fn generated_semantics_requires_source_aware_noop_completion() {
    let source = canonical_scaffold();
    assert!(
        source.contains("matchingAcceptedNoopIds(readyNoopItems"),
        "no-op verification must use source-aware artifact/evidence matching"
    );
    assert!(
        source.contains("matchingAcceptedCompletionIds(readyItems"),
        "wave completion repair must use source-aware matching"
    );
    assert!(
        !source.contains("matchingAcceptedIds(readyNoopItems"),
        "generic accepted matcher must not credit no-op source items"
    );
    assert!(
        !source.contains("matchingAcceptedIds(readyItems, completionEvidenceRepair"),
        "completion repair must not bypass source item requirements"
    );
}

fn canonical_scaffold() -> String {
    decomposed_prd_scaffold(
        "Implement decomposed PRD with dependency_ids",
        Some("/tmp/repo"),
        &task_universe(),
        &[],
        &archon_core::config::GeneratedWorkflowConfig::default(),
    )
    .expect("scaffold generation succeeds")
}

fn task_universe() -> WorkflowV2TaskUniverse {
    WorkflowV2TaskUniverse {
        schema_version: "workflow-v2-task-universe-v1".to_string(),
        source_roots: vec!["/tmp/tasks".to_string()],
        tasks: vec![
            WorkflowV2TaskUniverseTask {
                canonical_task_id: "TASK-TDL-001".to_string(),
                aliases: vec!["T001".to_string()],
                source_path: "/tmp/tasks/TASK-TDL-001.md".to_string(),
                dependency_ids: Vec::new(),
                title: None,
            },
            WorkflowV2TaskUniverseTask {
                canonical_task_id: "TASK-TDL-010".to_string(),
                aliases: vec!["T010".to_string()],
                source_path: "/tmp/tasks/TASK-TDL-010.md".to_string(),
                dependency_ids: vec!["TASK-TDL-001".to_string()],
                title: None,
            },
        ],
    }
}
