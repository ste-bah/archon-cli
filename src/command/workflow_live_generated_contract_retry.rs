fn normalize_retry_context(
    value: &serde_json::Value,
    object: &mut serde_json::Map<String, serde_json::Value>,
) {
    let mut commands = retry_step_values(
        value,
        &["command", "commands", "commands_run", "commandsRun"],
    );
    commands.extend(raw_values_from_aliases(
        value,
        &[
            "retry_command",
            "retryCommand",
            "retry_commands",
            "retryCommands",
            "recommended_retry",
            "recommendedRetry",
            "recommended_retries",
            "recommendedRetries",
        ],
    ));
    commands.extend(nested_alias_values(
        value,
        &["retry_command_shape", "retryCommandShape"],
        &["command", "commands"],
    ));
    commands.extend(nested_alias_values(
        value,
        &["commands_run_entry", "commandsRunEntry"],
        &["command", "commands"],
    ));
    commands.extend(nested_alias_values(
        value,
        &completion_evidence_roots(),
        &["command_refs", "commandRefs"],
    ));
    append_alias_values(object, "focused_verification", commands);
    let mut expected = retry_step_values(
        value,
        &[
            "required_evidence",
            "requiredEvidence",
            "evidence_to_capture",
            "evidenceToCapture",
            "expected_result",
            "expectedResult",
        ],
    );
    expected.extend(raw_values_from_aliases(
        value,
        &["acceptance_rule", "acceptanceRule"],
    ));
    if expected.is_empty() {
        expected.extend(raw_values_from_aliases(
            value,
            &["recommended_retry", "recommendedRetry"],
        ));
    }
    expected.extend(nested_alias_values(
        value,
        &completion_evidence_roots(),
        &[
            "evidence_refs",
            "evidenceRefs",
            "required_summary_points",
            "requiredSummaryPoints",
        ],
    ));
    append_alias_values(object, "expected_evidence", expected);
    let artifacts = nested_alias_values(
        value,
        &completion_evidence_roots(),
        &["artifact_paths", "artifactPaths"],
    );
    append_artifact_requirement_values(object, artifacts, false);
    if !object.contains_key("source_item_id")
        && let Some(source_item_id) =
            first_string(value, &["source_failed_item_id", "sourceFailedItemId"])
        {
            object.insert(
                "source_item_id".to_string(),
                serde_json::Value::String(source_item_id),
            );
        }
}

fn normalize_provider_env_context(
    value: &serde_json::Value,
    object: &mut serde_json::Map<String, serde_json::Value>,
) {
    let requirements = provider_env_requirement_values(value);
    append_alias_values(object, "provider_env_requirements", requirements.clone());
    append_alias_values(
        object,
        "expected_evidence",
        provider_env_expected_evidence(requirements),
    );
    copy_alias_value(
        value,
        &["provider_env_proof", "providerEnvProof"],
        object,
        "provider_env_proof",
    );
}

fn provider_env_requirement_aliases() -> [&'static str; 8] {
    [
        "provider_env_requirements",
        "providerEnvRequirements",
        "provider_env_required_keys",
        "providerEnvRequiredKeys",
        "required_env_keys",
        "requiredEnvKeys",
        "credential_env_keys",
        "credentialEnvKeys",
    ]
}

fn provider_env_expected_evidence(values: Vec<serde_json::Value>) -> Vec<serde_json::Value> {
    values
        .into_iter()
        .flat_map(value_to_strings)
        .map(|key| serde_json::json!(format!("provider_env_proof:{key}")))
        .collect()
}

fn provider_env_requirement_values(value: &serde_json::Value) -> Vec<serde_json::Value> {
    raw_values_from_aliases(value, &provider_env_requirement_aliases())
        .into_iter()
        .flat_map(value_to_strings)
        .map(|key| key.trim().to_ascii_uppercase())
        .filter(|key| !key.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(serde_json::Value::String)
        .collect()
}

fn collect_generated_inventory_items(value: &serde_json::Value) -> Vec<serde_json::Value> {
    if let Some(items) = top_level_items(value) {
        return items.clone();
    }
    let mut out = Vec::new();
    for root in generated_inventory_roots(value) {
        push_inventory_root_items(root, &mut out);
    }
    push_generated_retry_plan_items(value, &mut out);
    out
}

fn top_level_items(value: &serde_json::Value) -> Option<&Vec<serde_json::Value>> {
    value
        .get("items")
        .and_then(serde_json::Value::as_array)
}

fn generated_inventory_roots(value: &serde_json::Value) -> Vec<&serde_json::Value> {
    let mut roots = vec![value];
    for path in [
        &["data"][..],
        &["result"][..],
        &["result", "data"][..],
        &["result", "result"][..],
        &["result", "result", "data"][..],
        &["data", "result"][..],
        &["data", "result", "data"][..],
    ] {
        if let Some(root) = value_at_path(value, path) {
            roots.push(root);
        }
    }
    roots
}

fn value_at_path<'a>(
    value: &'a serde_json::Value,
    path: &[&str],
) -> Option<&'a serde_json::Value> {
    path.iter().try_fold(value, |current, key| current.get(*key))
}

fn push_inventory_root_items(root: &serde_json::Value, out: &mut Vec<serde_json::Value>) {
    push_item_collection(root.get("items"), out);
    push_item_collection(root.get("inventory").and_then(|v| v.get("items")), out);
    push_array(root.get("repaired_items"), out);
    push_array(root.get("implementation_items"), out);
    push_array(root.get("verified_noop_items"), out);
    if let Some(items) = root.get("items") {
        push_array(items.get("repaired_items"), out);
        push_array(items.get("implementation_items"), out);
        push_array(items.get("verified_noop_items"), out);
    }
}

fn push_item_collection(value: Option<&serde_json::Value>, out: &mut Vec<serde_json::Value>) {
    match value {
        Some(serde_json::Value::Array(items)) => out.extend(items.iter().cloned()),
        Some(serde_json::Value::Object(items)) => push_grouped_item_buckets(items, out),
        _ => {}
    }
}

fn push_grouped_item_buckets(
    items: &serde_json::Map<String, serde_json::Value>,
    out: &mut Vec<serde_json::Value>,
) {
    for key in [
        "implementation",
        "implementations",
        "implementation_items",
        "remediation",
        "remediation_items",
        "review_remediation",
        "review_remediation_items",
        "focused_verification",
        "focused_verification_items",
        "review_verification",
        "review_verification_items",
        "verified_noop",
        "verified_noops",
        "verified_noop_items",
    ] {
        push_array(items.get(key), out);
    }
}

fn push_array(value: Option<&serde_json::Value>, out: &mut Vec<serde_json::Value>) {
    if let Some(items) = value.and_then(serde_json::Value::as_array) {
        out.extend(items.iter().cloned());
    }
}

fn completion_evidence_roots() -> [&'static str; 4] {
    [
        "required_completion_evidence",
        "requiredCompletionEvidence",
        "completion_evidence_shape",
        "completionEvidenceShape",
    ]
}

fn push_generated_retry_plan_items(value: &serde_json::Value, out: &mut Vec<serde_json::Value>) {
    for root in generated_inventory_roots(value) {
        push_array(root.get("retry_items"), out);
        push_array(root.get("retryItems"), out);
        if let Some(plan) = root.get("repair_plan").or_else(|| root.get("repairPlan")) {
            push_array(plan.get("retry_items"), out);
            push_array(plan.get("retryItems"), out);
        }
    }
}

fn retry_step_values(value: &serde_json::Value, aliases: &[&str]) -> Vec<serde_json::Value> {
    ["retry_steps", "retrySteps", "retry_plan", "retryPlan"]
        .iter()
        .filter_map(|key| value.get(*key))
        .flat_map(|steps| match steps {
            serde_json::Value::Array(items) => items.clone(),
            serde_json::Value::Null => Vec::new(),
            other => vec![other.clone()],
        })
        .flat_map(|step| raw_values_from_aliases(&step, aliases))
        .collect()
}

fn nested_alias_values(
    value: &serde_json::Value,
    roots: &[&str],
    aliases: &[&str],
) -> Vec<serde_json::Value> {
    roots
        .iter()
        .filter_map(|root| value.get(*root))
        .flat_map(|root| match root {
            serde_json::Value::Array(items) => items
                .iter()
                .flat_map(|item| raw_values_from_aliases(item, aliases))
                .collect::<Vec<_>>(),
            _ => raw_values_from_aliases(root, aliases),
        })
        .collect()
}
