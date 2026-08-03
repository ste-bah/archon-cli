use super::*;

pub(crate) fn canonical_task_ids_from_generated_value(
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

pub(crate) fn normalize_canonical_ids(
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

pub(super) fn raw_canonical_task_ids_from_generated_value(
    value: &serde_json::Value,
) -> Vec<String> {
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

pub(crate) fn evidence_refs_from_generated_value(value: &serde_json::Value) -> Vec<String> {
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
pub(crate) fn lifecycle_target_file_issue(
    target: &str,
    target_repository_root: Option<&str>,
) -> Option<&'static str> {
    let root = target_repository_root.and_then(normalized_contract_path)?;
    target_file_issue(target, &root)
}

pub(crate) fn lifecycle_inventory_source_items(
    value: &serde_json::Value,
) -> Vec<serde_json::Value> {
    collect_generated_inventory_items(value)
}
