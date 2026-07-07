use archon_workflow::{
    WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2HostOptions, WorkflowV2WriteMode,
};

use super::super::workflow_live_generated_scaffold::decomposed_prd_scaffold;
use super::*;

#[test]
fn generated_semantics_ignores_simple_edit_workflow() {
    validate_generated_workflow_semantics(
        "Fix one bug",
        None,
        "export default async function workflow(w) { await w.implementation(\"fix\", { write: \"serial\", targetFiles: [\"src/lib.rs\"] }); }",
        &[],
    )
    .expect("simple workflows are out of scope");
}

#[test]
fn generated_semantics_accepts_deterministic_decomposed_prd_scaffold() {
    let source = canonical_scaffold();
    let calls = validated_calls(&source);

    assert!(source.contains("const targetRepositoryRoot = \"/tmp/repo\";"));
    assert!(source.contains("outside target repository root"));
    assert!(source.contains("task context/progress/report/artifact files"));
    assert!(source.contains("let dependencyIteration = 1;"));
    assert!(source.contains("let implementationWaveIndex = 1;"));
    assert!(source.contains("write: \"worktree\""));
    assert!(!source.contains("write: \"coordinated\""));
    assert!(source.contains("w.reduce(\"dependency-graph-repair-\" + repairAttempt"));
    assert!(
        source.contains("w.reduce(\"dependency-graph-repair-deadlock-\" + dependencyIteration")
    );
    assert!(
        source.contains(
            "remediationInventory = normalizeRemediationInventoryForSources(remediationInventory, readyImplementationItems, [], \"implementation-wave-\" + currentImplementationWaveIndex);"
        )
    );
    assert!(source.contains("while (!remediationInventoryReady(remediationInventory)"));
    assert!(source.contains(
        "remediationInventory = normalizeRemediationInventoryForSources(remediationInventoryRepair, readyImplementationItems, remediationInventory.items, \"implementation-wave-\" + currentImplementationWaveIndex);"
    ));
    assert!(source.contains(
        "followupRemediationInventory = filterRemediationInventoryByTaskIds(normalizeRemediationInventoryForSources(followupRemediationInventory, remediationInventory.items, readyImplementationItems, \"remediation-wave-\" + currentImplementationWaveIndex), remediationTaskIds);"
    ));
    assert!(source.contains(
        "completionEvidenceRepair = normalizeGeneratedInventory(completionEvidenceRepair);"
    ));
    assert!(source.contains("newlyCompletedIds = matchingAcceptedCompletionIds(readyItems, completionEvidenceRepair.items || completionEvidenceRepair.outcomes)"));
    assert!(source.contains("outcome.completion_evidence"));
    assert!(source.contains("outcome.artifact_paths"));
    assert!(source.contains("outcome.task_coverage"));

    validate_generated_workflow_semantics(
        "Implement decomposed PRD with dependency_ids",
        Some(&task_universe()),
        &source,
        &calls,
    )
    .expect("deterministic scaffold validates");
}

#[test]
fn generated_scaffold_emits_only_worktree_isolated_write_fanout() {
    // No source parser re-derives write modes any more: the scaffold
    // generator is the single producer, so assert its output directly, and
    // assert the declared plan agrees.
    let source = canonical_scaffold();
    assert!(source.contains("write: \"worktree\""));
    assert!(!source.contains("write: \"coordinated\""));
    assert!(!source.contains("write: \"serial\""));
    for call in super::super::workflow_live_generated_scaffold::decomposed_prd_plan_calls() {
        if call.method == WorkflowV2HostMethod::Fanout && call.write_mode.is_some() {
            assert_eq!(
                call.write_mode,
                Some(archon_workflow::WorkflowV2WriteMode::Worktree),
                "{}",
                call.id
            );
        }
    }
}

#[test]
fn generated_semantics_rejects_provider_authored_unordered_implementation_wave() {
    let err = validate_generated_workflow_semantics(
        "Implement decomposed PRD with dependency_ids",
        Some(&task_universe()),
        r#"export default async function workflow(w) {
            const inventory = await w.reduce("inventory", { outputs: ["items"] });
            const implementationItems = inventory.items;
            await w.fanout("implementation-wave-1", implementationItems, { itemKind: "implementation", write: "coordinated", targetFilesFromItem: true });
        }"#,
        &[implementation_call(
            "implementation-wave-1",
            Some("implementationItems"),
            None,
        )],
    )
    .expect_err("unordered provider-authored wave must be rejected");

    assert!(
        err.to_string()
            .contains("generated decomposed PRD workflow must")
    );
}

#[test]
fn generated_semantics_rejects_old_empty_every_completion_bug() {
    let source = canonical_scaffold().replace(
        "remainingItems = remainingItems.filter((item) => !itemIsCompleted(item, completedIds));",
        "remainingItems = remainingItems.filter((item) => !(item.canonical_task_ids || []).every((id) => completedIds.has(id)));",
    );
    let calls = validated_calls(&source);

    let err = validate_generated_workflow_semantics(
        "Implement decomposed PRD with dependency_ids",
        Some(&task_universe()),
        &source,
        &calls,
    )
    .expect_err("empty canonical_task_ids must not count as completed");

    assert!(err.to_string().contains("empty every"));
}

#[test]
fn generated_semantics_rejects_missing_per_wave_verification() {
    let source = canonical_scaffold().replace(
        "let verification = await w.parallel(\"verification-wave-\" + currentImplementationWaveIndex, verificationPlan.items,",
        "const verification = { status: \"accepted\", outcomes: [] };\n    await w.parallel(\"post-loop-verification-\" + currentImplementationWaveIndex, verificationPlan.items,",
    );
    let calls = validated_calls(&source);

    let err = validate_generated_workflow_semantics(
        "Implement decomposed PRD with dependency_ids",
        Some(&task_universe()),
        &source,
        &calls,
    )
    .expect_err("per-wave verification before unblocking dependents is required");

    assert!(err.to_string().contains("verification"));
}

#[test]
fn generated_semantics_rejects_retry_only_verification_lifecycle() {
    let source = canonical_scaffold().replace(
        "verification-failure-triage-\" + currentImplementationWaveIndex",
        "verification-retry-only-triage-\" + currentImplementationWaveIndex",
    );
    let calls = validated_calls(&source);

    let err = validate_generated_workflow_semantics(
        "Implement decomposed PRD with dependency_ids",
        Some(&task_universe()),
        &source,
        &calls,
    )
    .expect_err("actionable verification failures must route through triage/remediation");

    assert!(
        err.to_string()
            .contains("triage failed focused verification")
    );
}

#[test]
fn generated_semantics_scaffold_consumes_verification_retry_items() {
    let source = canonical_scaffold();

    assert!(
        source.contains(
            "let verificationRepairInventory = normalizeGeneratedInventory(verificationRepairPlan)"
        ),
        "verification repair reducer output must be normalized as a whole result"
    );
    assert!(
        source.contains("verification-repair-shape-repair-"),
        "repairable malformed verification reducer output must route through script-owned shape repair"
    );
    assert!(
        source.contains("verificationRepairPlan.items = generatedContractVerificationItems(verificationRepairInventory)"),
        "retry items must pass through the generated contract preflight before fanout"
    );
    assert!(
        !source.contains("verificationRepairPlan.retry_items"),
        "generated scaffold must not manually scrape retry_items outside the contract normalizer"
    );
}

#[test]
fn generated_semantics_scaffold_constrains_verification_repair_scope() {
    let source = canonical_scaffold();

    assert!(
        source.contains("const verificationRepairAllowedTaskIds"),
        "verification repair must derive the allowed task scope from the original verification plan"
    );
    assert!(
        source
            .matches("generatedContractConstrainInventoryTasks")
            .count()
            >= 2,
        "verification repair and shape repair must both be constrained before scheduling"
    );
}

#[test]
fn generated_semantics_rejects_reassigned_const_host_call_result() {
    let source = canonical_scaffold().replace(
        "let postRemediationVerificationPlan = await w.reduce(",
        "const postRemediationVerificationPlan = await w.reduce(",
    );
    let calls = validated_calls(&source);

    let err = validate_generated_workflow_semantics(
        "Implement decomposed PRD with dependency_ids",
        Some(&task_universe()),
        &source,
        &calls,
    )
    .expect_err("const host-call result rebinding must be rejected");

    assert!(
        err.to_string()
            .contains("must not reassign const host-call results")
    );
}

#[test]
fn generated_semantics_allows_mutable_host_call_result_reassignment() {
    let source = canonical_scaffold();
    let calls = validated_calls(&source);

    validate_generated_workflow_semantics(
        "Implement decomposed PRD with dependency_ids",
        Some(&task_universe()),
        &source,
        &calls,
    )
    .expect("let host-call result normalization is allowed");
}

#[test]
fn generated_semantics_allows_const_host_call_property_write() {
    let source = canonical_scaffold().replace(
        "let implementationEvidence = [];",
        "const benignHostResult = await w.reduce(\"benign-property-write\", [], { tier: \"reducer\" });\n  benignHostResult.items = [];\n  let implementationEvidence = [];",
    );
    let calls = validated_calls(&source);

    validate_generated_workflow_semantics(
        "Implement decomposed PRD with dependency_ids",
        Some(&task_universe()),
        &source,
        &calls,
    )
    .expect("property writes on const host-call results are allowed");
}

#[test]
fn generated_semantics_ignores_const_rebinding_text_in_strings_and_comments() {
    let source = canonical_scaffold().replace(
        "postRemediationVerificationPlan = normalizeGeneratedInventory(postRemediationVerificationPlan);",
        "\"postRemediationVerificationPlan = not code\";\n          // postRemediationVerificationPlan = not code\n          postRemediationVerificationPlan = normalizeGeneratedInventory(postRemediationVerificationPlan);",
    );
    let calls = validated_calls(&source);

    validate_generated_workflow_semantics(
        "Implement decomposed PRD with dependency_ids",
        Some(&task_universe()),
        &source,
        &calls,
    )
    .expect("string and comment text must not trigger const rebinding rejection");
}

#[test]
fn generated_scaffold_exposes_project_artifact_root_when_discoverable() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project_root = temp.path().join("project");
    std::fs::create_dir_all(project_root.join(".archon")).expect("archon dir");
    let task_path = project_root.join("tasks").join("TASK-TDL-001.md");
    let mut universe = task_universe();
    universe.source_roots = vec![project_root.join("tasks").display().to_string()];
    universe.tasks[0].source_path = task_path.display().to_string();

    let source = decomposed_prd_scaffold(
        "Implement decomposed PRD with dependency_ids",
        Some("/tmp/repo"),
        &universe,
        &[],
        &archon_core::config::GeneratedWorkflowConfig::default(),
    )
    .expect("scaffold");

    assert!(source.contains(&format!(
        "const projectArtifactRoot = \"{}\";",
        project_root.display()
    )));
    assert!(source.contains("projectArtifactRoot"));
}

#[test]
fn generated_semantics_accepts_direct_wave_outcomes_remediation_source() {
    let source = canonical_scaffold().replace(
        "w.reduce(\"remediation-inventory-\" + currentImplementationWaveIndex, [taskUniverse, readyImplementationItems, wave, failedImplementationOutcomes, implementationEvidence],",
        "w.reduce(\"remediation-inventory-\" + currentImplementationWaveIndex, wave.outcomes,",
    );
    let calls = validated_calls(&source);

    validate_generated_workflow_semantics(
        "Implement decomposed PRD with dependency_ids",
        Some(&task_universe()),
        &source,
        &calls,
    )
    .expect("direct wave.outcomes remediation source validates");
}

#[test]
fn generated_semantics_rejects_remediation_source_not_derived_from_wave_outcomes() {
    let source = canonical_scaffold().replace(
        "w.reduce(\"remediation-inventory-\" + currentImplementationWaveIndex, [taskUniverse, readyImplementationItems, wave, failedImplementationOutcomes, implementationEvidence],",
        "w.reduce(\"remediation-inventory-\" + currentImplementationWaveIndex, [taskUniverse, readyImplementationItems, wave, inventory.items, implementationEvidence],",
    );
    let calls = validated_calls(&source);

    let err = validate_generated_workflow_semantics(
        "Implement decomposed PRD with dependency_ids",
        Some(&task_universe()),
        &source,
        &calls,
    )
    .expect_err("unrelated remediation source must be rejected");

    assert!(
        err.to_string()
            .contains("route non-accepted implementation wave outcomes")
    );
}

#[test]
fn generated_semantics_rejects_remediation_fanout_without_preflight() {
    let source = canonical_scaffold()
        .replace(
            "while (!remediationInventoryReady(remediationInventory)",
            "while ((!remediationInventory.items || remediationInventory.items.length === 0)",
        )
        .replace(
            "remediationInventory = normalizeRemediationInventoryForSources(remediationInventory, readyImplementationItems, [], \"implementation-wave-\" + currentImplementationWaveIndex);",
            "remediationInventory = normalizeGeneratedInventory(remediationInventory);",
        );
    let calls = validated_calls(&source);

    let err = validate_generated_workflow_semantics(
        "Implement decomposed PRD with dependency_ids",
        Some(&task_universe()),
        &source,
        &calls,
    )
    .expect_err("remediation source metadata must be repaired before fanout");

    assert!(err.to_string().contains("preflight"));
}

#[test]
fn generated_semantics_rejects_dependency_deadlock_without_graph_repair() {
    let source = remove_between(
        canonical_scaffold(),
        "      let deadlockRepairAttempt = 1;",
        "      return await w.finalReport(\"blocked-dependency-deadlock-\"",
    );
    let calls = validated_calls(&source);

    let err = validate_generated_workflow_semantics(
        "Implement decomposed PRD with dependency_ids",
        Some(&task_universe()),
        &source,
        &calls,
    )
    .expect_err("deadlock graph repair must be required");

    assert!(err.to_string().contains("dependency graph repair"));
}

fn remove_between(mut source: String, start: &str, end: &str) -> String {
    let start_index = source
        .find(start)
        .unwrap_or_else(|| panic!("missing source start marker: {start}"));
    let end_index = source[start_index..]
        .find(end)
        .map(|index| start_index + index)
        .unwrap_or_else(|| panic!("missing source end marker: {end}"));
    source.replace_range(start_index..end_index, "");
    source
}

#[test]
fn generated_semantics_rejects_filtered_source_without_status_exclusion() {
    let source = canonical_scaffold().replace(
        "const failedImplementationOutcomes = nonAcceptedOutcomes(wave.outcomes);",
        "const failedImplementationOutcomes = (wave.outcomes || []).filter((outcome) => outcome);",
    );
    let calls = validated_calls(&source);

    let err = validate_generated_workflow_semantics(
        "Implement decomposed PRD with dependency_ids",
        Some(&task_universe()),
        &source,
        &calls,
    )
    .expect_err("filter that does not exclude accepted/noop must be rejected");

    assert!(
        err.to_string()
            .contains("route non-accepted implementation wave outcomes")
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

fn validated_calls(_source: &str) -> Vec<WorkflowV2HostCall> {
    super::super::workflow_live_generated_scaffold::decomposed_prd_plan_calls()
}

fn implementation_call(
    id: &str,
    source: Option<&str>,
    dynamic_prefix: Option<&str>,
) -> WorkflowV2HostCall {
    let mut extra = std::collections::BTreeMap::new();
    if let Some(prefix) = dynamic_prefix {
        extra.insert(
            "dynamic_id_prefix".to_string(),
            serde_json::Value::String(prefix.to_string()),
        );
    }
    WorkflowV2HostCall {
        id: id.to_string(),
        method: WorkflowV2HostMethod::Fanout,
        write_mode: Some(WorkflowV2WriteMode::Worktree),
        options: WorkflowV2HostOptions {
            source: source.map(str::to_string),
            item_kind: Some("implementation".to_string()),
            target_files_from_item: true,
            extra,
            ..WorkflowV2HostOptions::default()
        },
    }
}

fn task_universe() -> super::WorkflowV2TaskUniverse {
    super::WorkflowV2TaskUniverse {
        schema_version: "workflow-v2-task-universe-v1".to_string(),
        source_roots: vec!["/tmp/tasks".to_string()],
        tasks: vec![
            super::super::workflow_live_task_universe::WorkflowV2TaskUniverseTask {
                canonical_task_id: "TASK-TDL-001".to_string(),
                aliases: vec!["T001".to_string()],
                source_path: "/tmp/tasks/TASK-TDL-001.md".to_string(),
                dependency_ids: Vec::new(),
                title: None,
            },
            super::super::workflow_live_task_universe::WorkflowV2TaskUniverseTask {
                canonical_task_id: "TASK-TDL-010".to_string(),
                aliases: vec!["T010".to_string()],
                source_path: "/tmp/tasks/TASK-TDL-010.md".to_string(),
                dependency_ids: vec!["TASK-TDL-001".to_string()],
                title: None,
            },
        ],
    }
}
