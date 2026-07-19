fn copy_alias_array(
    value: &serde_json::Value,
    aliases: &[&str],
    object: &mut serde_json::Map<String, serde_json::Value>,
    target: &str,
) {
    let values = raw_values_from_aliases(value, aliases);
    if !values.is_empty() || aliases.iter().any(|key| value.get(*key).is_some()) {
        object.insert(target.to_string(), serde_json::Value::Array(values));
    }
}

fn copy_alias_value(
    value: &serde_json::Value,
    aliases: &[&str],
    object: &mut serde_json::Map<String, serde_json::Value>,
    target: &str,
) {
    if let Some(value) = aliases.iter().find_map(|key| value.get(*key).cloned()) {
        object.insert(target.to_string(), value);
    }
}

fn copy_target_file_aliases(
    value: &serde_json::Value,
    object: &mut serde_json::Map<String, serde_json::Value>,
) {
    copy_alias_array(value, TARGET_FILE_ALIASES, object, "target_files");
}

const TARGET_FILE_ALIASES: &[&str] = &[
    "target_files",
    "targetFiles",
    "files",
    "changed_files",
    "changedFiles",
    "owned_source_files",
    "ownedSourceFiles",
    "owned_test_files",
    "ownedTestFiles",
    "owned_manifest_files",
    "ownedManifestFiles",
    "owned_lockfiles",
    "ownedLockfiles",
    "owned_build_config_files",
    "ownedBuildConfigFiles",
    "owned_docs_config_files",
    "ownedDocsConfigFiles",
    "owned_generated_outputs",
    "ownedGeneratedOutputs",
];

fn copy_nested_required_evidence_array(
    value: &serde_json::Value,
    aliases: &[&str],
    object: &mut serde_json::Map<String, serde_json::Value>,
    target: &str,
) {
    let Some(required_evidence) = value
        .get("required_evidence")
        .or_else(|| value.get("requiredEvidence"))
        .or_else(|| value.get("expected_completion_evidence"))
        .or_else(|| value.get("expectedCompletionEvidence"))
    else {
        return;
    };
    let values = raw_values_from_aliases(required_evidence, aliases);
    if !values.is_empty()
        || aliases
            .iter()
            .any(|key| required_evidence.get(*key).is_some())
    {
        append_alias_values(object, target, values);
    }
}

fn copy_nested_object_array(
    value: &serde_json::Value,
    object_aliases: &[&str],
    aliases: &[&str],
    object: &mut serde_json::Map<String, serde_json::Value>,
    target: &str,
) {
    let values = object_aliases
        .iter()
        .filter_map(|key| value.get(*key))
        .flat_map(|nested| raw_values_from_aliases(nested, aliases))
        .collect::<Vec<_>>();
    if !values.is_empty() {
        append_alias_values(object, target, values);
    }
}

fn append_alias_values(
    object: &mut serde_json::Map<String, serde_json::Value>,
    target: &str,
    values: Vec<serde_json::Value>,
) {
    let mut merged = object
        .get(target)
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    append_unique_values(&mut merged, values);
    if !merged.is_empty() {
        object.insert(target.to_string(), serde_json::Value::Array(merged));
    }
}

fn append_unique_values(
    merged: &mut Vec<serde_json::Value>,
    values: Vec<serde_json::Value>,
) {
    for value in values {
        if !merged.iter().any(|existing| existing == &value) {
            merged.push(value);
        }
    }
}

fn normalize_remediation_context(
    value: &serde_json::Value,
    object: &mut serde_json::Map<String, serde_json::Value>,
) {
    if !object.contains_key("source_item_id")
        && let Some(source_item_id) = first_string(
            value,
            &[
                "source_item_id",
                "sourceItemId",
                "source_issue_id",
                "sourceIssueId",
                "issue_id",
                "issueId",
                "gap_id",
                "gapId",
            ],
        )
        .or_else(|| first_string(value, &["id", "item_id", "task_id", "taskId"]))
        {
            object.insert(
                "source_item_id".to_string(),
                serde_json::Value::String(source_item_id),
            );
        }
    if !object.contains_key("failure_status") {
        if let Some(status) = first_string(
            value,
            &[
                "failure_status",
                "failureStatus",
                "status",
                "blocker_status",
                "blockerStatus",
            ],
        ) {
            object.insert(
                "failure_status".to_string(),
                serde_json::Value::String(status),
            );
        } else if value.get("acceptance_blocker").is_some()
            || value.get("blocker").is_some()
            || value.get("required_evidence").is_some()
        {
            object.insert(
                "failure_status".to_string(),
                serde_json::Value::String("needs_review".to_string()),
            );
        }
    }
    if !object.contains_key("failure_evidence") {
        let evidence = raw_values_from_aliases(
            value,
            &[
                "failure_evidence",
                "failureEvidence",
                "failure_kind",
                "failureKind",
                "verification_failure_class",
                "verificationFailureClass",
                "evidence",
                "acceptance_blocker",
                "acceptanceBlocker",
                "blocker",
                "residual_gaps",
                "residualGaps",
                "gaps",
            ],
        );
        if !evidence.is_empty() {
            object.insert(
                "failure_evidence".to_string(),
                serde_json::Value::Array(evidence),
            );
        }
    }
    if !object.contains_key("required_fix")
        && let Some(required_fix) = first_string(
            value,
            &[
                "required_fix",
                "requiredFix",
                "fix",
                "remediation",
                "title",
                "summary",
                "acceptance_blocker",
                "acceptanceBlocker",
                "blocker",
            ],
        ) {
            object.insert(
                "required_fix".to_string(),
                serde_json::Value::String(required_fix),
            );
        }
    if !object.contains_key("verification_requirements") {
        let requirements = raw_values_from_aliases(
            value,
            &[
                "verification_requirements",
                "verificationRequirements",
                "verification_shape",
                "verificationShape",
                "verification",
                "focused_tests",
                "focusedTests",
            ],
        );
        if !requirements.is_empty() {
            object.insert(
                "verification_requirements".to_string(),
                serde_json::Value::Array(requirements),
            );
        }
    }
}

fn first_string(value: &serde_json::Value, aliases: &[&str]) -> Option<String> {
    aliases
        .iter()
        .find_map(|key| value.get(*key))
        .and_then(string_value)
}

fn string_value(value: &serde_json::Value) -> Option<String> {
    value
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn raw_strings_from_aliases(value: &serde_json::Value, aliases: &[&str]) -> Vec<String> {
    sorted_unique(
        raw_values_from_aliases(value, aliases)
            .into_iter()
            .flat_map(value_to_strings)
            .collect(),
    )
}

fn embedded_task_ids_from_generated_value(
    value: &serde_json::Value,
    universe: &ContractTaskUniverse,
) -> Vec<String> {
    let haystack = raw_strings_from_aliases(
        value,
        &[
            "item_id",
            "id",
            "source_item_id",
            "sourceItemId",
            "failed_item_id",
            "failedItemId",
            "source_failed_item_id",
            "sourceFailedItemId",
        ],
    )
    .join(" ")
    .to_ascii_lowercase();
    sorted_unique(
        universe
            .canonical
            .iter()
            .filter(|id| {
                embedded_task_candidates(id)
                    .iter()
                    .any(|candidate| haystack.contains(candidate))
            })
            .cloned()
            .collect(),
    )
}

fn embedded_task_candidates(canonical: &str) -> Vec<String> {
    let mut candidates = vec![canonical.to_ascii_lowercase()];
    if let Some(short) = short_task_alias(canonical) {
        candidates.push(short.to_ascii_lowercase());
    }
    let parts = canonical.split('-').collect::<Vec<_>>();
    if parts.len() > 2 {
        candidates.push(parts[1..].join("-").to_ascii_lowercase());
    }
    sorted_unique(candidates)
}

fn raw_values_from_aliases(value: &serde_json::Value, aliases: &[&str]) -> Vec<serde_json::Value> {
    aliases
        .iter()
        .filter_map(|key| value.get(*key))
        .flat_map(|value| match value {
            serde_json::Value::Array(values) => values.clone(),
            serde_json::Value::Null => Vec::new(),
            other => vec![other.clone()],
        })
        .collect()
}

fn value_to_strings(value: serde_json::Value) -> Vec<String> {
    match value {
        serde_json::Value::String(value) => value
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect(),
        serde_json::Value::Object(object) => object
            .get("path")
            .or_else(|| object.get("id"))
            .or_else(|| object.get("summary"))
            .or_else(|| object.get("command"))
            .and_then(serde_json::Value::as_str)
            .map(|value| vec![value.trim().to_string()])
            .unwrap_or_default(),
        serde_json::Value::Number(value) => vec![value.to_string()],
        serde_json::Value::Bool(value) => vec![value.to_string()],
        _ => Vec::new(),
    }
}

fn value_present(value: Option<&serde_json::Value>) -> bool {
    match value {
        Some(serde_json::Value::String(value)) => !value.trim().is_empty(),
        Some(serde_json::Value::Array(values)) => !values.is_empty(),
        Some(serde_json::Value::Object(values)) => !values.is_empty(),
        Some(serde_json::Value::Bool(_)) | Some(serde_json::Value::Number(_)) => true,
        _ => false,
    }
}

fn dedupe_issues(issues: Vec<GeneratedContractIssue>) -> Vec<GeneratedContractIssue> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for issue in issues {
        let key = (
            issue.kind.clone(),
            issue.field.clone(),
            issue.message.clone(),
            issue.item_id.clone(),
            issue.canonical_task_ids.clone(),
        );
        if seen.insert(key) {
            out.push(issue);
        }
    }
    out
}

fn sorted_unique(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn short_task_alias(canonical: &str) -> Option<String> {
    let digits = canonical.rsplit('-').next()?;
    (!digits.is_empty() && digits.chars().all(|ch| ch.is_ascii_digit()))
        .then(|| format!("T{digits}"))
}
