use super::super::workflow_live_generated_scaffold::decomposed_prd_scaffold;
use super::super::workflow_live_task_universe::{
    WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask,
};
use super::validate_generated_workflow_semantics;

#[test]
fn generated_scaffold_repairs_review_remediation_inventory_before_fanout() {
    let source = canonical_scaffold();
    let calls = super::super::workflow_live_generated_scaffold::decomposed_prd_plan_calls();

    assert!(source.contains("normalizeReviewRemediationInventory"));
    assert!(source.contains("reviewNeedsRemediation(review)"));
    assert!(source.contains("reviewRemediationInput(review)"));
    assert!(source.contains("review-remediation-inventory-repair-"));
    assert!(source.contains("Do not invent synthetic canonical task IDs."));
    assert!(source.contains("target_files to []"));

    validate_generated_workflow_semantics(
        "Implement decomposed PRD with dependency_ids",
        Some(&task_universe()),
        &source,
        &calls,
    )
    .expect("scaffold with review remediation preflight validates");
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
        tasks: vec![task("TASK-001", &[]), task("TASK-010", &["TASK-001"])],
    }
}

fn task(id: &str, dependencies: &[&str]) -> WorkflowV2TaskUniverseTask {
    WorkflowV2TaskUniverseTask {
        canonical_task_id: id.to_string(),
        aliases: Vec::new(),
        source_path: format!("/tmp/tasks/{id}.md"),
        dependency_ids: dependencies.iter().map(|id| (*id).to_string()).collect(),
        title: None,
    }
}
