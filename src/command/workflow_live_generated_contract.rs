use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::workflow_live_task_universe::WorkflowV2TaskUniverse;

#[path = "workflow_live_generated_contract_universe.rs"]
mod contract_universe;
use contract_universe::ContractTaskUniverse;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum GeneratedContractIssueKind {
    InventoryShapeRepair,
    TaskUniverseReconcile,
    DependencyGraphRepair,
    TargetFileDiscovery,
    VerificationRequirementsDiscovery,
    ArtifactRequirementsDiscovery,
    ProviderEnvironmentDiscovery,
    EvidenceRepair,
}

impl GeneratedContractIssueKind {
    #[cfg(test)]
    pub(super) fn as_str(&self) -> &'static str {
        match self {
            Self::InventoryShapeRepair => "inventory_shape_repair",
            Self::TaskUniverseReconcile => "task_universe_reconcile",
            Self::DependencyGraphRepair => "dependency_graph_repair",
            Self::TargetFileDiscovery => "target_file_discovery",
            Self::VerificationRequirementsDiscovery => "verification_requirements_discovery",
            Self::ArtifactRequirementsDiscovery => "artifact_requirements_discovery",
            Self::ProviderEnvironmentDiscovery => "provider_environment_discovery",
            Self::EvidenceRepair => "evidence_repair",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct GeneratedContractIssue {
    pub(super) kind: GeneratedContractIssueKind,
    pub(super) field: String,
    pub(super) message: String,
    pub(super) item_id: Option<String>,
    pub(super) canonical_task_ids: Vec<String>,
}

#[derive(Debug, Clone)]
pub(super) struct NormalizedGeneratedItem {
    pub(super) value: serde_json::Value,
    pub(super) issues: Vec<GeneratedContractIssue>,
}

#[derive(Debug, Clone)]
pub(super) struct NormalizedGeneratedInventory {
    pub(super) items: Vec<serde_json::Value>,
    pub(super) issues: Vec<GeneratedContractIssue>,
}

#[cfg(test)]
pub(super) fn normalize_generated_inventory_value(
    value: &serde_json::Value,
    task_universe: Option<&WorkflowV2TaskUniverse>,
) -> NormalizedGeneratedInventory {
    normalize_generated_inventory_value_with_repo(value, task_universe, None)
}

pub(super) fn normalize_generated_inventory_value_with_repo(
    value: &serde_json::Value,
    task_universe: Option<&WorkflowV2TaskUniverse>,
    target_repository_root: Option<&str>,
) -> NormalizedGeneratedInventory {
    let raw_items = collect_generated_inventory_items(value);
    let mut issues = Vec::new();
    let mut items = Vec::new();
    let universe = ContractTaskUniverse::from_authoritative(task_universe);
    if raw_items.is_empty() {
        issues.push(empty_inventory_issue(&universe));
    }
    for raw in raw_items {
        let normalized =
            normalize_generated_item_value_with_repo(&raw, task_universe, target_repository_root);
        if generated_support_item(&normalized.value) {
            continue;
        }
        issues.extend(normalized.issues);
        items.push(normalized.value);
    }
    issues.extend(generated_inventory_graph_issues(&items, &universe));
    NormalizedGeneratedInventory { items, issues }
}

fn empty_inventory_issue(universe: &ContractTaskUniverse) -> GeneratedContractIssue {
    GeneratedContractIssue {
        kind: GeneratedContractIssueKind::InventoryShapeRepair,
        field: "items".to_string(),
        message: "generated implementation inventory did not contain schedulable items".to_string(),
        item_id: None,
        canonical_task_ids: universe.canonical.iter().cloned().collect(),
    }
}

pub(super) fn normalize_generated_item_value(
    value: &serde_json::Value,
    task_universe: Option<&WorkflowV2TaskUniverse>,
) -> NormalizedGeneratedItem {
    normalize_generated_item_value_with_repo(value, task_universe, None)
}

pub(super) fn normalize_generated_item_value_with_repo(
    value: &serde_json::Value,
    task_universe: Option<&WorkflowV2TaskUniverse>,
    target_repository_root: Option<&str>,
) -> NormalizedGeneratedItem {
    let universe = ContractTaskUniverse::from_authoritative(task_universe);
    let mut object = value.as_object().cloned().unwrap_or_default();
    let item_id = first_string(
        value,
        &[
            "item_id",
            "itemId",
            "id",
            "task_id",
            "taskId",
            "work_unit_id",
            "workUnitId",
            "source_item_id",
            "sourceItemId",
            "failed_item_id",
            "failedItemId",
            "source_failed_item_id",
            "sourceFailedItemId",
        ],
    );
    if let Some(item_id) = &item_id {
        object.insert(
            "item_id".to_string(),
            serde_json::Value::String(item_id.clone()),
        );
        object.insert("id".to_string(), serde_json::Value::String(item_id.clone()));
    }
    if let Some(work_type) = first_string(
        value,
        &["work_type", "workType", "item_type", "itemType", "kind"],
    ) {
        object.insert("work_type".to_string(), serde_json::json!(work_type));
    }
    let canonical_task_ids = canonical_task_ids_from_generated_value(value, task_universe);
    if !canonical_task_ids.is_empty() {
        object.insert(
            "canonical_task_ids".to_string(),
            serde_json::json!(&canonical_task_ids),
        );
    }
    stamp_declared_capabilities(&mut object, &canonical_task_ids, task_universe);
    let dependency_ids = dependency_ids_from_generated_value(value, task_universe);
    object.insert(
        "dependency_ids".to_string(),
        serde_json::json!(dependency_ids.clone()),
    );
    copy_target_file_aliases(value, &mut object);
    copy_alias_array(
        value,
        &[
            "acceptance_criteria",
            "acceptanceCriteria",
            "criteria",
            "acceptance",
        ],
        &mut object,
        "acceptance_criteria",
    );
    copy_alias_array(
        value,
        &[
            "focused_verification",
            "focusedVerification",
            "focused_tests",
            "focusedTests",
            "verification",
            "verification_shape",
            "verificationShape",
            "command",
            "check",
            "test_command",
            "testCommand",
            "commands",
            "commands_run",
            "commandsRun",
            "manual_fixture_steps",
            "manualFixtureSteps",
        ],
        &mut object,
        "focused_verification",
    );
    if !value_present(object.get("focused_verification")) {
        append_alias_values(
            &mut object,
            "focused_verification",
            raw_values_from_aliases(
                value,
                &["verification_requirements", "verificationRequirements"],
            ),
        );
    }
    copy_nested_required_evidence_array(
        value,
        &[
            "focused_tests",
            "focusedTests",
            "direct_checks",
            "directChecks",
            "commands",
            "commands_run",
            "commandsRun",
            "required_summary_points",
            "requiredSummaryPoints",
        ],
        &mut object,
        "focused_verification",
    );
    copy_nested_object_array(
        value,
        &["manual_fixture_retry", "manualFixtureRetry"],
        &["commands", "commands_run", "commandsRun"],
        &mut object,
        "focused_verification",
    );
    copy_alias_array(
        value,
        &[
            "expected_evidence",
            "expectedEvidence",
            "expected_acceptance",
            "expectedAcceptance",
            "required_evidence",
            "requiredEvidence",
            "evidence_to_capture",
            "evidenceToCapture",
            "expected_result",
            "expectedResult",
        ],
        &mut object,
        "expected_evidence",
    );
    copy_artifact_requirement_aliases(value, &mut object);
    copy_nested_artifact_requirement_aliases(value, &mut object);
    copy_nested_object_artifact_aliases(
        value,
        &["manual_fixture_retry", "manualFixtureRetry"],
        &[
            "artifact_checks",
            "artifactChecks",
            "expected_artifacts",
            "expectedArtifacts",
            "artifact_paths",
            "artifactPaths",
        ],
        &mut object,
    );
    normalize_retry_context(value, &mut object);
    normalize_retry_invariant_context(value, &mut object);
    copy_alias_array(
        value,
        &[
            "noop_proof_refs",
            "noopProofRefs",
            "proof_references",
            "proofReferences",
            "proof_refs",
            "proofRefs",
        ],
        &mut object,
        "noop_proof_refs",
    );
    copy_alias_value(
        value,
        &[
            "noop_proof",
            "noopProof",
            "proof",
            "noop_evidence",
            "noopEvidence",
        ],
        &mut object,
        "noop_proof",
    );
    copy_alias_array(
        value,
        &[
            "provider_evidence",
            "providerEvidence",
            "provider_env_evidence",
            "providerEnvEvidence",
            "environment_evidence",
            "environmentEvidence",
        ],
        &mut object,
        "provider_evidence",
    );
    normalize_provider_env_context(value, &mut object);
    copy_alias_value(
        value,
        &["evidence", "proof", "proof_references", "proofReferences"],
        &mut object,
        "evidence",
    );
    normalize_remediation_context(value, &mut object);

    let normalized_value = serde_json::Value::Object(object);
    let issues = generated_item_issues(&normalized_value, &universe, target_repository_root);
    NormalizedGeneratedItem {
        value: normalized_value,
        issues,
    }
}

fn stamp_declared_capabilities(
    object: &mut serde_json::Map<String, serde_json::Value>,
    canonical_task_ids: &[String],
    task_universe: Option<&WorkflowV2TaskUniverse>,
) {
    let Some(task_universe) = task_universe else {
        return;
    };
    let selected = task_universe
        .tasks
        .iter()
        .filter(|task| canonical_task_ids.contains(&task.canonical_task_id));
    let mut required_env_keys = BTreeSet::new();
    let mut required_tools = BTreeSet::new();
    let mut deliverable_contracts = BTreeSet::new();
    for task in selected {
        required_env_keys.extend(task.required_env_keys.iter().cloned());
        required_tools.extend(task.required_tools.iter().cloned());
        deliverable_contracts.extend(task.deliverable_contracts.iter().cloned());
    }
    if !required_env_keys.is_empty() {
        object.insert(
            "required_env_keys".to_string(),
            serde_json::json!(required_env_keys),
        );
    }
    if !required_tools.is_empty() {
        object.insert(
            "required_tools".to_string(),
            serde_json::json!(required_tools),
        );
    }
    if !deliverable_contracts.is_empty() {
        object.insert(
            "deliverable_contracts".to_string(),
            serde_json::json!(deliverable_contracts),
        );
    }
}

pub(super) fn canonical_task_ids_from_generated_value(
    value: &serde_json::Value,
    task_universe: Option<&WorkflowV2TaskUniverse>,
) -> Vec<String> {
    let universe = ContractTaskUniverse::from_authoritative(task_universe);
    let explicit = sorted_unique(
        raw_strings_from_aliases(
            value,
            &[
                "canonical_task_ids",
                "canonicalTaskIds",
                "canonical_task_id",
                "canonicalTaskId",
                "task_ids",
                "taskIds",
                "task_id",
                "taskId",
            ],
        )
        .into_iter()
        .filter_map(|id| universe.resolve(&id))
        .collect(),
    );
    if !explicit.is_empty() {
        return explicit;
    }
    embedded_task_ids_from_generated_value(value, &universe)
}

pub(super) fn dependency_ids_from_generated_value(
    value: &serde_json::Value,
    task_universe: Option<&WorkflowV2TaskUniverse>,
) -> Vec<String> {
    let universe = ContractTaskUniverse::from_authoritative(task_universe);
    sorted_unique(
        raw_strings_from_aliases(
            value,
            &[
                "dependency_ids",
                "dependencyIds",
                "dependencies",
                "depends_on",
                "dependsOn",
                "canonical_dependency_ids",
                "canonicalDependencyIds",
            ],
        )
        .into_iter()
        .map(|id| {
            universe
                .resolve(&id)
                .unwrap_or_else(|| format!("__unresolved__:{id}"))
        })
        .collect(),
    )
}

pub(super) fn evidence_refs_from_generated_value(value: &serde_json::Value) -> Vec<String> {
    sorted_unique(
        raw_strings_from_aliases(
            value,
            &[
                "evidence",
                "evidence_refs",
                "evidenceRefs",
                "proof_references",
                "proofReferences",
                "proof_refs",
                "proofRefs",
                "noop_proof_refs",
                "noopProofRefs",
                "artifacts",
                "artifact_paths",
                "artifactPaths",
                "command",
                "commands",
                "commands_run",
                "commandsRun",
                "files_changed",
                "filesChanged",
            ],
        )
        .into_iter()
        .collect(),
    )
}

/// Lifecycle shims: JS `generatedContractTargetFileIssue` and the item-less
/// fallback of JS `generatedContractInventorySourceItems`.
pub(super) fn lifecycle_target_file_issue(
    target: &str,
    target_repository_root: Option<&str>,
) -> Option<&'static str> {
    let root = target_repository_root.and_then(normalized_contract_path)?;
    target_file_issue(target, &root)
}

pub(super) fn lifecycle_inventory_source_items(
    value: &serde_json::Value,
) -> Vec<serde_json::Value> {
    collect_generated_inventory_items(value)
}

include!("workflow_live_generated_contract_validation.rs");

include!("workflow_live_generated_contract_helpers.rs");

include!("workflow_live_generated_contract_artifacts.rs");

include!("workflow_live_generated_contract_retry.rs");

include!("workflow_live_generated_contract_invariants.rs");

#[cfg(test)]
#[path = "workflow_live_generated_contract_tests.rs"]
mod tests;
