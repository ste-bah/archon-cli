use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::workflow_live_task_universe::WorkflowV2TaskUniverse;

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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct CanonicalIdNormalization {
    pub(super) canonical_ids: Vec<String>,
    pub(super) unresolved_ids: Vec<String>,
}

#[derive(Debug, Clone, Default)]
struct ContractTaskUniverse {
    canonical: BTreeSet<String>,
    aliases: BTreeMap<String, String>,
    dependencies: BTreeMap<String, Vec<String>>,
    tasks_with_deliverable_contracts: BTreeSet<String>,
}

impl ContractTaskUniverse {
    fn from_authoritative(task_universe: Option<&WorkflowV2TaskUniverse>) -> Self {
        let mut out = Self::default();
        let Some(task_universe) = task_universe else {
            return out;
        };
        for task in &task_universe.tasks {
            out.add_canonical(&task.canonical_task_id);
            for alias in &task.aliases {
                out.aliases
                    .insert(alias.trim().to_string(), task.canonical_task_id.clone());
            }
            out.dependencies.insert(
                task.canonical_task_id.clone(),
                sorted_unique(task.dependency_ids.clone()),
            );
            if !task.deliverable_contracts.is_empty() {
                out.tasks_with_deliverable_contracts
                    .insert(task.canonical_task_id.clone());
            }
        }
        out
    }

    fn has_deliverable_contract(&self, task_ids: &[String]) -> bool {
        task_ids
            .iter()
            .any(|id| self.tasks_with_deliverable_contracts.contains(id))
    }

    fn add_canonical(&mut self, task_id: &str) {
        let canonical = task_id.trim();
        if canonical.is_empty() {
            return;
        }
        self.canonical.insert(canonical.to_string());
        self.aliases
            .insert(canonical.to_string(), canonical.to_string());
        if let Some(short) = short_task_alias(canonical) {
            self.aliases.insert(short, canonical.to_string());
        }
    }

    fn resolve(&self, value: &str) -> Option<String> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return None;
        }
        if self.canonical.is_empty() {
            return Some(trimmed.to_string());
        }
        let mut matches = self
            .aliases
            .iter()
            .filter(|(alias, _)| alias.eq_ignore_ascii_case(trimmed))
            .map(|(_, canonical)| canonical.clone())
            .collect::<BTreeSet<_>>();
        for canonical in &self.canonical {
            let Some((_, suffix)) = canonical.split_once('-') else {
                continue;
            };
            if suffix.contains('-') && suffix.eq_ignore_ascii_case(trimmed) {
                matches.insert(canonical.clone());
            }
        }
        (matches.len() == 1)
            .then(|| matches.into_iter().next())
            .flatten()
    }

    fn dependencies_for(&self, task_id: &str) -> Vec<String> {
        self.dependencies.get(task_id).cloned().unwrap_or_default()
    }
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
    let raw_canonical_task_ids = raw_canonical_task_ids_from_generated_value(value);
    let canonical_id_normalization =
        normalize_canonical_ids(task_universe, raw_canonical_task_ids.clone());
    let canonical_task_ids = if canonical_id_normalization.canonical_ids.is_empty() {
        embedded_task_ids_from_generated_value(value, &universe)
    } else {
        canonical_id_normalization.canonical_ids.clone()
    };
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
    let mut issues = generated_item_issues(&normalized_value, &universe, target_repository_root);
    if !canonical_id_normalization.unresolved_ids.is_empty() {
        issues.push(GeneratedContractIssue {
            kind: GeneratedContractIssueKind::TaskUniverseReconcile,
            field: "canonical_task_ids".to_string(),
            message: format!(
                "inventory item has unresolvable canonical task IDs: {}",
                canonical_id_normalization.unresolved_ids.join(", ")
            ),
            item_id,
            canonical_task_ids: raw_canonical_task_ids,
        });
    }
    let issues = dedupe_issues(issues);
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
    let explicit = normalize_canonical_ids(
        task_universe,
        raw_canonical_task_ids_from_generated_value(value),
    )
    .canonical_ids;
    if !explicit.is_empty() {
        return explicit;
    }
    embedded_task_ids_from_generated_value(value, &universe)
}

pub(super) fn normalize_canonical_ids(
    task_universe: Option<&WorkflowV2TaskUniverse>,
    ids: impl IntoIterator<Item = String>,
) -> CanonicalIdNormalization {
    let universe = ContractTaskUniverse::from_authoritative(task_universe);
    let mut canonical_ids = BTreeSet::new();
    let mut unresolved_ids = BTreeSet::new();
    for id in ids {
        let id = id.trim();
        if id.is_empty() {
            continue;
        }
        match universe.resolve(id) {
            Some(canonical) => {
                canonical_ids.insert(canonical);
            }
            None => {
                unresolved_ids.insert(id.to_string());
            }
        }
    }
    CanonicalIdNormalization {
        canonical_ids: canonical_ids.into_iter().collect(),
        unresolved_ids: unresolved_ids.into_iter().collect(),
    }
}

fn raw_canonical_task_ids_from_generated_value(value: &serde_json::Value) -> Vec<String> {
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
