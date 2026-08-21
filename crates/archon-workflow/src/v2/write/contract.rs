use super::*;

pub(super) fn branch_results_from_outcomes(
    outcomes: &[WorkflowV2BranchOutcome],
) -> Vec<WorkflowV2Result> {
    outcomes
        .iter()
        .filter_map(|outcome| {
            let mut result = outcome.result.clone()?;
            tag_branch_result(&mut result, &outcome.item_id);
            Some(result)
        })
        .collect()
}

pub(super) fn tag_branch_result(result: &mut WorkflowV2Result, item_id: &str) {
    let mut object = match std::mem::take(&mut result.data) {
        serde_json::Value::Object(object) => object,
        serde_json::Value::Null => serde_json::Map::new(),
        value => {
            let mut object = serde_json::Map::new();
            object.insert("data".to_string(), value);
            object
        }
    };
    object
        .entry("item_id".to_string())
        .or_insert_with(|| serde_json::Value::String(item_id.to_string()));
    object.insert(
        "branch_id".to_string(),
        serde_json::Value::String(item_id.to_string()),
    );
    result.data = serde_json::Value::Object(object);
}

pub(super) fn normalize_write_branch_contract_result(result: &mut WorkflowV2Result) {
    if should_skip_write_contract_normalization(result) {
        return;
    }

    let item_id = item_id_from_branch_result(result);
    let canonical_task_ids = canonical_task_ids_from_branch_result(result);
    let evidence = concrete_evidence_from_branch_result(result);
    let contract_errors =
        write_branch_contract_errors(item_id.as_deref(), &canonical_task_ids, &evidence);
    if contract_errors.is_empty() {
        return;
    }

    mark_write_branch_contract_invalid(
        result,
        item_id.as_deref(),
        canonical_task_ids,
        evidence,
        contract_errors,
    );
}

pub(super) fn should_skip_write_contract_normalization(result: &WorkflowV2Result) -> bool {
    matches!(
        failure_kind_from_write_result(result),
        Some(BranchFailureKind::Safety | BranchFailureKind::Execution)
    ) || matches!(result.status, WorkflowV2Status::Cancelled)
}

pub(super) fn write_branch_contract_errors(
    item_id: Option<&str>,
    canonical_task_ids: &[String],
    evidence: &[serde_json::Value],
) -> Vec<String> {
    let mut contract_errors = Vec::new();
    if item_id.is_none() {
        contract_errors.push("missing item_id/id".to_string());
    }
    if canonical_task_ids.is_empty() {
        contract_errors.push("missing canonical_task_ids".to_string());
    }
    if evidence.is_empty() {
        contract_errors.push("missing concrete evidence".to_string());
    }
    contract_errors
}

pub(super) fn mark_write_branch_contract_invalid(
    result: &mut WorkflowV2Result,
    item_id: Option<&str>,
    canonical_task_ids: Vec<String>,
    evidence: Vec<serde_json::Value>,
    contract_errors: Vec<String>,
) {
    result.status = WorkflowV2Status::NeedsReview;
    add_write_branch_contract_review(result);
    add_write_branch_contract_gap(result, item_id, &contract_errors);
    set_write_branch_contract_data(result, canonical_task_ids, evidence, contract_errors);
}

pub(super) fn add_write_branch_contract_review(result: &mut WorkflowV2Result) {
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Review,
        "implementation branch outcome must include item_id/id, canonical_task_ids, status, and concrete evidence before it can unblock dependent work",
    ));
}

pub(super) fn add_write_branch_contract_gap(
    result: &mut WorkflowV2Result,
    item_id: Option<&str>,
    contract_errors: &[String],
) {
    result.residual_gaps.push(WorkflowV2ResidualGap {
        id: format!(
            "invalid_implementation_branch_contract_{}",
            sanitize_v2_path_segment(item_id.unwrap_or("unknown"))
        ),
        description: format!(
            "missing implementation branch outcome contract fields: {}",
            contract_errors.join(", ")
        ),
        severity: Some("review".to_string()),
    });
}

pub(super) fn set_write_branch_contract_data(
    result: &mut WorkflowV2Result,
    canonical_task_ids: Vec<String>,
    evidence: Vec<serde_json::Value>,
    contract_errors: Vec<String>,
) {
    let mut object = result_data_object(result);
    object.insert(
        "status".to_string(),
        serde_json::Value::String("needs_review".to_string()),
    );
    object.insert(
        "failure_kind".to_string(),
        serde_json::to_value(BranchFailureKind::Contract)
            .unwrap_or_else(|_| serde_json::Value::String("contract".to_string())),
    );
    object.insert("contract_valid".to_string(), serde_json::Value::Bool(false));
    object.insert(
        "contract_errors".to_string(),
        serde_json::to_value(contract_errors).unwrap_or(serde_json::Value::Null),
    );
    object
        .entry("canonical_task_ids".to_string())
        .or_insert_with(|| serde_json::to_value(canonical_task_ids).unwrap_or_default());
    object
        .entry("evidence".to_string())
        .or_insert_with(|| serde_json::to_value(evidence).unwrap_or_default());
    result.data = serde_json::Value::Object(object);
}

pub(super) fn result_data_object(
    result: &mut WorkflowV2Result,
) -> serde_json::Map<String, serde_json::Value> {
    match std::mem::take(&mut result.data) {
        serde_json::Value::Object(object) => object,
        serde_json::Value::Null => serde_json::Map::new(),
        value => {
            let mut object = serde_json::Map::new();
            object.insert("data".to_string(), value);
            object
        }
    }
}

pub(super) fn save_write_branch_outcome(
    v2_store: &WorkflowV2ResultStore,
    call_id: &str,
    item_id: &str,
    role: &str,
    item_input_hash: Option<String>,
    result: &WorkflowV2Result,
) -> crate::WorkflowResult<()> {
    let call = WorkflowV2HostCall {
        id: call_id.to_string(),
        method: crate::WorkflowV2HostMethod::Fanout,
        write_mode: Some(WorkflowV2WriteMode::Coordinated),
        options: Default::default(),
    };
    let mut outcome = WorkflowV2BranchOutcome {
        item_id: item_id.to_string(),
        role: role.to_string(),
        status: result.status,
        result: Some(result.clone()),
        error: None,
        failure_kind: failure_kind_from_write_result(result),
        item_input_hash,
        completion_evidence: Vec::new(),
    };
    attach_completion_evidence_for_call(&call, &mut outcome);
    v2_store.save_branch_outcome(call_id, &outcome)?;
    Ok(())
}

pub(super) fn write_items_for_branches(
    target_repository_root: Option<&str>,
    call: &WorkflowV2HostCall,
    branches: &[crate::WorkflowV2FanoutItem],
) -> crate::WorkflowResult<Vec<WorkflowV2WriteItem>> {
    let mode = call.write_mode.unwrap_or(WorkflowV2WriteMode::Serial);
    branches
        .iter()
        .map(|branch| {
            let expanded = expanded_targets_for_branch(target_repository_root, call, branch)?;
            if expanded.target_files.is_empty() {
                // An artifact-only branch still needs ownership scopes, or its
                // agent is told it may write nothing and refuses the very
                // deliverable it was dispatched to produce (observed live:
                // TDL-001 blocked three runs with "target_files and ownership
                // scopes are empty"). Grant the directories of the artifacts it
                // is contracted to produce; the write-safety layer still rejects
                // anything outside them.
                let scopes = artifact_ownership_scopes(branch);
                Ok(WorkflowV2WriteItem::artifact_only(branch.id.clone(), mode)
                    .with_owned_scopes(scopes))
            } else {
                Ok(
                    WorkflowV2WriteItem::new(branch.id.clone(), mode, expanded.target_files)
                        .with_owned_scopes(expanded.target_dir_scopes),
                )
            }
        })
        .collect()
}

#[cfg(test)]
pub(super) fn target_files_for_branch(
    target_repository_root: Option<&str>,
    call: &WorkflowV2HostCall,
    branch: &crate::WorkflowV2FanoutItem,
) -> crate::WorkflowResult<Vec<String>> {
    Ok(expanded_targets_for_branch(target_repository_root, call, branch)?.target_files)
}

pub(super) fn expanded_targets_for_branch(
    target_repository_root: Option<&str>,
    call: &WorkflowV2HostCall,
    branch: &crate::WorkflowV2FanoutItem,
) -> crate::WorkflowResult<ExpandedTargetFiles> {
    let branch_targets = &branch.call.options.target_files;
    if !branch_targets.is_empty() && branch_targets != &call.options.target_files {
        return expand_declared_targets(&branch.id, branch_targets, target_repository_root);
    }
    if call.options.target_files_from_item {
        let targets = target_files_from_branch_item(branch);
        if !targets.is_empty() {
            return expand_declared_targets(&branch.id, &targets, target_repository_root);
        }
    }
    if !call.options.target_files.is_empty() {
        return expand_declared_targets(
            &branch.id,
            &call.options.target_files,
            target_repository_root,
        );
    }
    if branch_has_artifact_requirements(branch) {
        return Ok(ExpandedTargetFiles {
            declared_target_files: Vec::new(),
            target_files: Vec::new(),
            target_dir_scopes: Vec::new(),
            target_file_expansions: Vec::new(),
        });
    }
    Err(WorkflowError::SpecInvalid(format!(
        "write-capable fanout '{}' item '{}' has no target file ownership",
        call.id, branch.id
    )))
}

pub(super) fn target_files_from_branch_item(branch: &crate::WorkflowV2FanoutItem) -> Vec<String> {
    branch
        .input
        .get("item")
        .and_then(|item| {
            item.get("target_files")
                .or_else(|| item.get("expected_target_files"))
        })
        .and_then(serde_json::Value::as_array)
        .map(|items| target_file_strings(items))
        .unwrap_or_default()
}

/// Directory scopes an artifact-only branch owns: the parent directory of
/// each declared artifact requirement path. Reads the same `artifact_requirements`
/// field `item_has_artifact_requirements` gates on, so a branch that qualified
/// as artifact-only necessarily yields a non-empty scope set unless every
/// declared path is a bare filename. Skips templated/absolute/glob paths, which
/// cannot name a safe directory.
pub(super) fn artifact_ownership_scopes(branch: &crate::WorkflowV2FanoutItem) -> Vec<String> {
    let Some(item) = branch.input.get("item") else {
        return Vec::new();
    };
    let mut scopes: Vec<String> = Vec::new();
    for key in ["artifact_requirements", "project_artifact_requirements"] {
        for path in artifact_requirement_strings(item.get(key)) {
            if let Some(dir) = artifact_parent_scope(&path)
                && !scopes.contains(&dir)
            {
                scopes.push(dir);
            }
        }
    }
    scopes
}

fn artifact_requirement_strings(value: Option<&serde_json::Value>) -> Vec<String> {
    match value {
        Some(serde_json::Value::Array(values)) => values
            .iter()
            .filter_map(|entry| match entry {
                serde_json::Value::String(text) => Some(text.trim().to_string()),
                serde_json::Value::Object(map) => map
                    .get("path")
                    .or_else(|| map.get("artifact_path"))
                    .and_then(serde_json::Value::as_str)
                    .map(|text| text.trim().to_string()),
                _ => None,
            })
            .filter(|text| !text.is_empty())
            .collect(),
        Some(serde_json::Value::String(text)) if !text.trim().is_empty() => {
            vec![text.trim().to_string()]
        }
        _ => Vec::new(),
    }
}

fn artifact_parent_scope(raw: &str) -> Option<String> {
    if raw.contains("${")
        || raw.contains('*')
        || raw.contains('<')
        || std::path::Path::new(raw).is_absolute()
    {
        return None;
    }
    let segments: Vec<&str> = raw
        .split('/')
        .filter(|segment| !segment.is_empty() && *segment != ".")
        .collect();
    if segments.contains(&"..") || segments.len() < 2 {
        return None;
    }
    Some(segments[..segments.len() - 1].join("/"))
}

pub(super) fn target_file_strings(items: &[serde_json::Value]) -> Vec<String> {
    items
        .iter()
        .filter_map(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

pub(super) fn branch_has_artifact_requirements(branch: &crate::WorkflowV2FanoutItem) -> bool {
    branch
        .input
        .get("item")
        .is_some_and(item_has_artifact_requirements)
}

pub(super) fn item_has_artifact_requirements(item: &serde_json::Value) -> bool {
    ["artifact_requirements", "project_artifact_requirements"]
        .iter()
        .any(|key| item.get(*key).is_some_and(value_has_content))
}

pub(super) fn value_has_content(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::String(value) => !value.trim().is_empty(),
        serde_json::Value::Array(values) => values.iter().any(value_has_content),
        serde_json::Value::Object(values) => values.values().any(value_has_content),
        _ => !value.is_null(),
    }
}

pub(super) fn expand_declared_targets(
    item_id: &str,
    targets: &[String],
    target_repository_root: Option<&str>,
) -> crate::WorkflowResult<ExpandedTargetFiles> {
    expand_declared_rust_module_targets(item_id, targets, target_repository_root)
        .map_err(|err| WorkflowError::SpecInvalid(err.to_string()))
}
