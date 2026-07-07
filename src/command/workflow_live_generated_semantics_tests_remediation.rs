use super::super::workflow_live_generated_scaffold::decomposed_prd_scaffold;
use super::super::workflow_live_task_universe::{
    WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask,
};
use super::validate_generated_workflow_semantics;

#[test]
fn generated_scaffold_filters_followup_remediation_to_original_task_scope() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/wf835_followup_remediation_side_task.json"
    ))
    .expect("fixture json");
    assert_eq!(fixture["original_unresolved_task_ids"][0], "TASK-TDL-010");

    let source = canonical_scaffold();
    assert!(source.contains("function remediationTaskIdSet(items)"));
    assert!(
        source.contains(
            "const remediationTaskIds = remediationTaskIdSet(remediationInventory.items);"
        )
    );
    assert!(source.contains("followupRemediationInventory = filterRemediationInventoryByTaskIds(normalizeRemediationInventoryForSources(followupRemediationInventory, remediationInventory.items, readyImplementationItems, \"remediation-wave-\" + currentImplementationWaveIndex), remediationTaskIds);"));
}

#[test]
fn generated_scaffold_preserves_source_ownership_for_remediation() {
    let source = canonical_scaffold();
    assert!(source.contains("[taskUniverse, readyImplementationItems, wave, failedImplementationOutcomes, implementationEvidence]"));
    assert!(source.contains("normalizeRemediationInventoryForSources(remediationInventory, readyImplementationItems, [], \"implementation-wave-\" + currentImplementationWaveIndex);"));
    assert!(source.contains("normalizeRemediationInventoryForSources(remediationInventoryRepair, readyImplementationItems, remediationInventory.items, \"implementation-wave-\" + currentImplementationWaveIndex);"));
    assert!(source.contains("[taskUniverse, readyImplementationItems, remediationInventory.items, remediationWave, unresolvedAfterRemediation]"));
    assert!(source.contains(
        "function remediationSourceForItem(item, sourceItems, fallbackItems, sourceCallId)"
    ));
}

#[test]
fn generated_semantics_rejects_unscoped_followup_remediation_inventory() {
    let source = canonical_scaffold().replace(
        "followupRemediationInventory = filterRemediationInventoryByTaskIds(normalizeRemediationInventoryForSources(followupRemediationInventory, remediationInventory.items, readyImplementationItems, \"remediation-wave-\" + currentImplementationWaveIndex), remediationTaskIds);",
        "followupRemediationInventory = normalizeRemediationInventory(followupRemediationInventory);",
    );
    let calls = super::super::workflow_live_generated_scaffold::decomposed_prd_plan_calls();

    let err = validate_generated_workflow_semantics(
        "Implement decomposed PRD with dependency_ids",
        Some(&task_universe()),
        &source,
        &calls,
    )
    .expect_err("unscoped follow-up remediation must be rejected");

    assert!(err.to_string().contains("follow-up remediation"));
}

#[test]
fn generated_semantics_rejects_remediation_without_source_ownership() {
    let source = canonical_scaffold().replace(
        "remediationInventory = normalizeRemediationInventoryForSources(remediationInventory, readyImplementationItems, [], \"implementation-wave-\" + currentImplementationWaveIndex);",
        "remediationInventory = normalizeRemediationInventory(remediationInventory);",
    );
    let calls = super::super::workflow_live_generated_scaffold::decomposed_prd_plan_calls();

    let err = validate_generated_workflow_semantics(
        "Implement decomposed PRD with dependency_ids",
        Some(&task_universe()),
        &source,
        &calls,
    )
    .expect_err("remediation without source ownership must be rejected");

    assert!(err.to_string().contains("preserve original ownership"));
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
