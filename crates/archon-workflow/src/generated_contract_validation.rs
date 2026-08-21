use super::*;

pub(super) fn generated_item_issues(
    value: &serde_json::Value,
    universe: &ContractTaskUniverse,
    target_repository_root: Option<&str>,
) -> Vec<GeneratedContractIssue> {
    let mut issues = Vec::new();
    if generated_support_item(value) {
        return issues;
    }
    let item_id = first_string(value, &["item_id", "id"]);
    let canonical_task_ids = raw_strings_from_aliases(value, &["canonical_task_ids"]);
    let work_type = first_string(value, &["work_type", "workType"]).unwrap_or_default();
    let make_issue =
        |kind: GeneratedContractIssueKind, field: &str, message: &str| -> GeneratedContractIssue {
            GeneratedContractIssue {
                kind,
                field: field.to_string(),
                message: message.to_string(),
                item_id: item_id.clone(),
                canonical_task_ids: canonical_task_ids.clone(),
            }
        };
    if item_id.is_none() {
        issues.push(make_issue(
            GeneratedContractIssueKind::InventoryShapeRepair,
            "item_id",
            "inventory item is missing item_id/id",
        ));
    }
    if canonical_task_ids.is_empty() {
        issues.push(make_issue(
            GeneratedContractIssueKind::InventoryShapeRepair,
            "canonical_task_ids",
            "inventory item is missing canonical task IDs",
        ));
    }
    for task_id in &canonical_task_ids {
        if !universe.canonical.is_empty() && !universe.canonical.contains(task_id) {
            issues.push(make_issue(
                GeneratedContractIssueKind::TaskUniverseReconcile,
                "canonical_task_ids",
                "inventory item has task IDs outside the canonical task universe",
            ));
        }
    }
    for dep in raw_strings_from_aliases(value, &["dependency_ids"]) {
        if dep.starts_with("__unresolved__:")
            || (!universe.canonical.is_empty() && universe.resolve(&dep).is_none())
        {
            issues.push(make_issue(
                GeneratedContractIssueKind::TaskUniverseReconcile,
                "dependency_ids",
                "inventory item has dependency IDs outside the canonical task universe",
            ));
        }
    }
    if value_present(value.get("artifact_requirement_issues")) {
        issues.push(make_issue(
            GeneratedContractIssueKind::ArtifactRequirementsDiscovery,
            "artifact_requirements",
            "artifact declarations must be concrete paths or explicit evidence expectations",
        ));
    }
    if generated_focused_verification_item(value) {
        for field in retry_invariant_missing_fields(value) {
            issues.push(make_issue(
                GeneratedContractIssueKind::EvidenceRepair,
                field,
                "verification retry item must preserve source residual gap IDs and the failed predicate",
            ));
        }
        return issues;
    }
    if !universe.canonical.is_empty()
        && !value_present(value.get("target_files"))
        && !universe.has_deliverable_contract(&canonical_task_ids)
        && !explicit_no_artifact_verified_noop(value, &work_type)
        && artifact_producing_intent(value)
    {
        issues.push(make_issue(
            GeneratedContractIssueKind::ArtifactRequirementsDiscovery,
            "deliverable_contracts",
            "task produces artifacts but declares no deliverable contract and no target_files",
        ));
    }
    // ONE item, ONE deliverable contract.
    //
    // The reason tasks silently never fire. Observed live: an inventory emitted
    // one item claiming four canonical tasks and another claiming three. Three
    // of that first group never appeared in any implementation wave, because
    // they had no item of their own to be dispatched as. Acceptance is per
    // ITEM, so one agent turn, one result and one no-op proof retired four
    // separate contracts, and every rule that made acceptance stricter still
    // only ever had one story to judge.
    //
    // Grouping itself was already policed in three directions — a task grouped
    // with its own prerequisite, a task claimed by two items, a task claimed by
    // none — and the repair that splits a grouped item already exists. Nothing
    // triggered it. The inventory prompt asks for splits; asking is not a rule.
    //
    // Scoped to tasks that declare a deliverable contract, because that is
    // exactly where the harm is: each contract needs its own proof. Tasks with
    // no contract of their own may still share an item.
    //
    // It applies to BOTH work types, and the first version of this rule did not.
    // It was scoped to `implementation` on the reasoning that the refuted-no-op
    // and execution rules already policed no-ops — which was wrong: those decide
    // whether a no-op is ALLOWED, not whether one item may cover four tasks.
    // The very next run emitted a no-op item claiming four contracted tasks,
    // and this rule watched it go past.
    if matches!(work_type.as_str(), "implementation" | "verified_noop")
        && universe.contracted_task_count(&canonical_task_ids) > 1
    {
        issues.push(make_issue(
            GeneratedContractIssueKind::InventoryShapeRepair,
            "canonical_task_ids",
            "this item claims more than one canonical task that declares its own \
             deliverable contract: split it so each contracted task has an item, \
             or one result will close all of them",
        ));
    }
    match work_type.as_str() {
        "implementation" => {
            if let Some(message) = target_files_issue(value, target_repository_root) {
                issues.push(make_issue(
                    GeneratedContractIssueKind::TargetFileDiscovery,
                    "target_files",
                    &message,
                ));
            }
            if !value_present(value.get("acceptance_criteria")) {
                issues.push(make_issue(
                    GeneratedContractIssueKind::VerificationRequirementsDiscovery,
                    "acceptance_criteria",
                    "implementation item is missing acceptance criteria",
                ));
            }
            if !value_present(value.get("focused_verification")) {
                issues.push(make_issue(
                    GeneratedContractIssueKind::VerificationRequirementsDiscovery,
                    "focused_verification",
                    "implementation item is missing focused verification requirements",
                ));
            }
            if value.get("artifact_requirements").is_none() {
                issues.push(make_issue(
                    GeneratedContractIssueKind::ArtifactRequirementsDiscovery,
                    "artifact_requirements",
                    "implementation item is missing artifact requirements",
                ));
            }
        }
        "verified_noop" => verified_noop_issues(
            value,
            universe,
            &canonical_task_ids,
            &mut issues,
            &make_issue,
        ),
        _ => issues.push(make_issue(
            GeneratedContractIssueKind::InventoryShapeRepair,
            "work_type",
            "inventory item work_type must be implementation or verified_noop",
        )),
    }
    if value
        .get("provider_required")
        .and_then(serde_json::Value::as_bool)
        == Some(true)
        && !value_present(value.get("provider_evidence"))
        && !value_present(value.get("provider_env_requirements"))
        && !value_present(value.get("provider_env_proof"))
    {
        issues.push(make_issue(
            GeneratedContractIssueKind::ProviderEnvironmentDiscovery,
            "provider_evidence",
            "provider-dependent item is missing provider/environment evidence",
        ));
    }
    issues
}

fn explicit_no_artifact_verified_noop(value: &serde_json::Value, work_type: &str) -> bool {
    work_type == "verified_noop"
        && value
            .get("artifact_requirements")
            .and_then(serde_json::Value::as_array)
            .is_some_and(Vec::is_empty)
}

fn artifact_producing_intent(value: &serde_json::Value) -> bool {
    if value_present(value.get("artifact_requirements")) || value_present(value.get("artifacts")) {
        return true;
    }
    let text = [
        "acceptance_criteria",
        "expected_evidence",
        "required_fix",
        "noop_proof",
        "noop_proof_refs",
        "evidence",
        "summary",
    ]
    .into_iter()
    .filter_map(|key| value.get(key))
    .map(serde_json::Value::to_string)
    .collect::<Vec<_>>()
    .join(" ")
    .to_ascii_lowercase();
    let artifact_noun = [
        "artifact",
        "report",
        "registry",
        "manifest",
        "dataset",
        "output file",
    ]
    .iter()
    .any(|term| text.contains(term));
    let production_or_claim = [
        "produce", "write", "create", "generate", "persist", "update", "emit", "exists", "present",
        "missing", "absent",
    ]
    .iter()
    .any(|term| text.contains(term));
    artifact_noun && production_or_claim
}

/// task_id -> claiming (item_id, canonical_task_ids) pairs.
type TaskClaims = BTreeMap<String, Vec<(Option<String>, Vec<String>)>>;
pub(super) fn generated_support_item(value: &serde_json::Value) -> bool {
    let work_type = first_string(value, &["work_type", "workType", "kind"])
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !work_type.ends_with("_support") {
        return false;
    }
    raw_strings_from_aliases(value, &["canonical_task_ids"]).is_empty()
}

pub(super) fn generated_inventory_graph_issues(
    items: &[serde_json::Value],
    universe: &ContractTaskUniverse,
) -> Vec<GeneratedContractIssue> {
    if universe.canonical.is_empty() {
        return Vec::new();
    }
    if !items.is_empty() && items.iter().all(generated_focused_verification_item) {
        return Vec::new();
    }
    let mut issues = Vec::new();
    let mut claimed_by_task: TaskClaims = BTreeMap::new();
    for item in items {
        let item_id = first_string(item, &["item_id", "id"]);
        let canonical_task_ids = raw_strings_from_aliases(item, &["canonical_task_ids"]);
        for task_id in &canonical_task_ids {
            claimed_by_task
                .entry(task_id.clone())
                .or_default()
                .push((item_id.clone(), canonical_task_ids.clone()));
        }
        let claimed = canonical_task_ids.iter().cloned().collect::<BTreeSet<_>>();
        for dep in raw_strings_from_aliases(item, &["dependency_ids"]) {
            if claimed.contains(&dep) {
                issues.push(GeneratedContractIssue {
                    kind: GeneratedContractIssueKind::DependencyGraphRepair,
                    field: "dependency_ids".to_string(),
                    message: format!(
                        "inventory item dependency '{dep}' is also claimed by the same item"
                    ),
                    item_id: item_id.clone(),
                    canonical_task_ids: canonical_task_ids.clone(),
                });
            }
        }
        for task_id in &canonical_task_ids {
            for dep in universe.dependencies_for(task_id) {
                if claimed.contains(&dep) {
                    issues.push(GeneratedContractIssue {
                        kind: GeneratedContractIssueKind::DependencyGraphRepair,
                        field: "canonical_task_ids".to_string(),
                        message: format!(
                            "inventory item groups '{task_id}' with its prerequisite '{dep}'"
                        ),
                        item_id: item_id.clone(),
                        canonical_task_ids: canonical_task_ids.clone(),
                    });
                }
            }
        }
    }

    for (task_id, claims) in &claimed_by_task {
        if claims.len() > 1 {
            for (item_id, canonical_task_ids) in claims {
                issues.push(GeneratedContractIssue {
                    kind: GeneratedContractIssueKind::DependencyGraphRepair,
                    field: "canonical_task_ids".to_string(),
                    message: format!(
                        "canonical task '{task_id}' is assigned to multiple inventory items"
                    ),
                    item_id: item_id.clone(),
                    canonical_task_ids: canonical_task_ids.clone(),
                });
            }
        }
    }

    let all_claimed = claimed_by_task.keys().cloned().collect::<BTreeSet<_>>();
    for task_id in &universe.canonical {
        if !all_claimed.contains(task_id) {
            issues.push(GeneratedContractIssue {
                kind: GeneratedContractIssueKind::DependencyGraphRepair,
                field: "canonical_task_ids".to_string(),
                message: format!(
                    "canonical task '{task_id}' is not represented by an implementation or verified_noop item"
                ),
                item_id: None,
                canonical_task_ids: vec![task_id.clone()],
            });
        }
    }
    for item in items {
        let item_id = first_string(item, &["item_id", "id"]);
        let canonical_task_ids = raw_strings_from_aliases(item, &["canonical_task_ids"]);
        let mut required = BTreeSet::new();
        for task_id in &canonical_task_ids {
            required.extend(universe.dependencies_for(task_id));
        }
        required.extend(raw_strings_from_aliases(item, &["dependency_ids"]));
        for dep in required {
            if dep.starts_with("__unresolved__:") || !universe.canonical.contains(&dep) {
                continue;
            }
            if !all_claimed.contains(&dep) {
                issues.push(GeneratedContractIssue {
                    kind: GeneratedContractIssueKind::DependencyGraphRepair,
                    field: "dependency_ids".to_string(),
                    message: format!(
                        "inventory dependency '{dep}' is not represented by an implementation or verified_noop item"
                    ),
                    item_id: item_id.clone(),
                    canonical_task_ids: canonical_task_ids.clone(),
                });
            }
        }
    }

    dedupe_issues(issues)
}

pub(super) fn generated_focused_verification_item(value: &serde_json::Value) -> bool {
    generated_verification_intent(value)
        && value_present(value.get("focused_verification"))
        && (value.get("expected_evidence").is_some()
            || value.get("artifact_requirements").is_some())
        && !value_present(value.get("target_files"))
}

fn generated_verification_intent(value: &serde_json::Value) -> bool {
    let work_type = first_string(value, &["work_type", "workType"])
        .unwrap_or_default()
        .to_ascii_lowercase();
    if work_type.is_empty() || work_type.contains("verification") {
        return true;
    }
    work_type == "verified_noop" && generated_retry_verification_intent(value)
}

fn generated_retry_verification_intent(value: &serde_json::Value) -> bool {
    [
        "retry_command_shape",
        "retryCommandShape",
        "retry_steps",
        "retrySteps",
    ]
    .iter()
    .any(|key| value_present(value.get(*key)))
        || [
            "expected_result_shape",
            "expectedResultShape",
            "retry_plan",
            "retryPlan",
        ]
        .iter()
        .any(|key| value_present(value.get(*key)))
}

fn target_files_issue(
    value: &serde_json::Value,
    target_repository_root: Option<&str>,
) -> Option<String> {
    let targets = raw_strings_from_aliases(value, &["target_files"]);
    if targets.is_empty() {
        // Empty is legitimate for artifact-only work — a task whose every
        // deliverable is a project artifact has no repository file to name,
        // and raising an issue here created one no repair could ever resolve:
        // the reducer cannot invent a repo path the write layer would accept,
        // so the issue stayed unresolved and bricked the inventory. Same rule
        // as `valid_inventory_item` and artifact-only remediation: [] is valid
        // exactly when the artifact requirements are concrete.
        if value_present(value.get("artifact_requirements")) {
            return None;
        }
        return Some("implementation item is missing target files".to_string());
    }
    let root = target_repository_root.and_then(normalized_contract_path)?;
    for target in targets {
        if let Some(message) = target_file_issue(&target, &root) {
            return Some(format!("{message}: {target}"));
        }
    }
    None
}

pub(super) fn target_file_issue(target: &str, root: &str) -> Option<&'static str> {
    let trimmed = target.trim();
    if trimmed.is_empty() {
        return Some("implementation item has an empty target file");
    }
    if trimmed.chars().any(char::is_whitespace) {
        return Some("target_files must hold repository file paths, not instruction text or prose");
    }
    let normalized = normalized_contract_path(trimmed)?;
    let absolute = is_contract_absolute_path(trimmed);
    if absolute {
        if normalized == root {
            return Some(
                "implementation target_files must name repo-owned files, not the repository root",
            );
        }
        let root_prefix = format!("{root}/");
        if !normalized.starts_with(&root_prefix) {
            return Some("implementation target file is outside target repository root");
        }
    } else if normalized == "." || normalized == ".." || normalized.starts_with("../") {
        return Some("implementation target file escapes target repository root");
    }
    None
}

pub(super) fn normalized_contract_path(path: &str) -> Option<String> {
    let raw = path.trim().replace('\\', "/");
    if raw.is_empty() {
        return None;
    }
    let absolute = is_contract_absolute_path(&raw);
    let mut parts = Vec::new();
    for part in raw.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.last().is_some_and(|last| *last != "..") {
                    parts.pop();
                } else if !absolute {
                    parts.push("..");
                }
            }
            other => parts.push(other),
        }
    }
    if absolute {
        if parts.is_empty() {
            Some("/".to_string())
        } else {
            Some(format!("/{}", parts.join("/")))
        }
    } else if parts.is_empty() {
        Some(".".to_string())
    } else {
        Some(parts.join("/"))
    }
}

fn is_contract_absolute_path(path: &str) -> bool {
    path.starts_with('/') || {
        let mut chars = path.chars();
        matches!(
            (chars.next(), chars.next(), chars.next()),
            (Some(drive), Some(':'), Some('\\' | '/')) if drive.is_ascii_alphabetic()
        )
    }
}
