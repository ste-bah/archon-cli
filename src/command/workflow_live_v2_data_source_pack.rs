use super::*;

pub(super) const SOURCE_PACK_TEXT_LIMIT: usize = 700;

pub(in super::super) fn source_pack_value(value: &serde_json::Value) -> serde_json::Value {
    // Packed reduce source is prior agent output, and allowed_mcp_tools scans
    // the whole stage input: a tool declaration surviving the pack would bind
    // MCP tools on the reducer. The unknown-object packer preserves every key,
    // so strip tool declarations here — the fanout branch builder already does
    // the same for write/verify items.
    let mut value = value.clone();
    super::super::super::workflow_live_mcp::strip_tool_declarations(&mut value);
    let value = &value;
    match value {
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(source_pack_value).collect())
        }
        serde_json::Value::Object(object) => {
            if object.contains_key("status") && object.contains_key("summary") {
                return pack_result_like_object(object);
            }
            let mut packed = serde_json::Map::new();
            for (key, value) in object {
                if key == "result"
                    || key == "items"
                    || key.ends_with("Items")
                    || key.ends_with("_items")
                {
                    packed.insert(key.clone(), source_pack_value(value));
                } else if key == "branch_artifact_paths" || key == "artifact_paths" {
                    packed.insert(key.clone(), value.clone());
                } else if key == "outcomes" {
                    packed.insert("outcomes".to_string(), pack_outcomes(value));
                    packed.insert("outcome_count".to_string(), json_array_len(value));
                } else if is_large_text_field(key, value) {
                    packed.insert(key.clone(), truncate_json_text(value));
                } else {
                    packed.insert(key.clone(), compact_unknown_source_value(value));
                }
            }
            serde_json::Value::Object(packed)
        }
        other => compact_unknown_source_value(other),
    }
}

pub(super) fn pack_outcomes(value: &serde_json::Value) -> serde_json::Value {
    let Some(outcomes) = value.as_array() else {
        return serde_json::Value::Array(Vec::new());
    };
    serde_json::Value::Array(outcomes.iter().map(pack_outcome).collect())
}

pub(super) fn pack_outcome(value: &serde_json::Value) -> serde_json::Value {
    let Some(object) = value.as_object() else {
        return compact_unknown_source_value(value);
    };
    let mut packed = serde_json::Map::new();
    // D74 companion: the triage/repair prompts order reducers to preserve gap
    // identity and classification — so the packer must not strip those fields
    // from their inputs.
    for key in [
        "item_id",
        "id",
        "source_item_id",
        "canonical_task_ids",
        "dependency_ids",
        "status",
        "failure_kind",
        "failure_status",
        "failure_evidence",
        "failed_predicate",
        "source_residual_gap_ids",
        "classification",
        "verification_failure_class",
        "pass_fail_count",
        "matched_test_check_names",
        "error",
        "expected_evidence",
        "focused_verification",
        "recommended_retry",
        "provider_env_proof",
        "acceptance_criteria_results",
        "summary",
        "evidence",
        "completion_evidence",
        "artifact_paths",
        "artifacts",
        "commands_run",
        "files_read",
        "files_changed",
        "task_coverage",
        "residual_gaps",
    ] {
        if let Some(value) = object.get(key) {
            packed.insert(key.to_string(), compact_known_result_field(key, value));
        }
    }
    serde_json::Value::Object(packed)
}

pub(super) fn pack_result_like_object(
    object: &serde_json::Map<String, serde_json::Value>,
) -> serde_json::Value {
    let mut packed = serde_json::Map::new();
    // Result-like objects can BE outcomes (top-level status+summary); keep
    // their identity and verification-semantic fields alongside the result
    // projection so triage inputs never lose what routing must preserve.
    for key in [
        "status",
        "summary",
        "item_id",
        "id",
        "source_item_id",
        "canonical_task_ids",
        "failure_kind",
        "failed_predicate",
        "source_residual_gap_ids",
        "classification",
        "verification_failure_class",
        "pass_fail_count",
        "matched_test_check_names",
        "error",
        "provider_env_proof",
        "evidence",
        "artifacts",
        "commands_run",
        "files_read",
        "files_changed",
        "task_coverage",
        "residual_gaps",
    ] {
        if let Some(value) = object.get(key) {
            packed.insert(key.to_string(), compact_known_result_field(key, value));
        }
    }
    if let Some(data) = object.get("data") {
        if let Some(items) = data.get("items") {
            packed.insert("items".to_string(), source_pack_value(items));
        }
        if let Some(outcomes) = data.get("outcomes") {
            packed.insert("outcomes".to_string(), pack_outcomes(outcomes));
            packed.insert("outcome_count".to_string(), json_array_len(outcomes));
        }
        if let Some(paths) = data.get("branch_artifact_paths") {
            packed.insert("branch_artifact_paths".to_string(), paths.clone());
        }
    }
    if let Some(outcomes) = object.get("outcomes") {
        packed.insert("outcomes".to_string(), pack_outcomes(outcomes));
        packed.insert("outcome_count".to_string(), json_array_len(outcomes));
    }
    serde_json::Value::Object(packed)
}

pub(super) fn compact_known_result_field(key: &str, value: &serde_json::Value) -> serde_json::Value {
    match key {
        "summary" => truncate_json_text(value),
        "evidence" => serde_json::Value::Array(
            value
                .as_array()
                .into_iter()
                .flatten()
                .map(|evidence| {
                    serde_json::json!({
                        "kind": evidence.get("kind"),
                        "summary": evidence.get("summary").map(truncate_json_text),
                        "source": evidence.get("source"),
                    })
                })
                .collect(),
        ),
        "commands_run" => serde_json::Value::Array(
            value
                .as_array()
                .into_iter()
                .flatten()
                .map(|command| {
                    serde_json::json!({
                        "kind": command.get("kind"),
                        "command": command.get("command"),
                        "status": command.get("status"),
                        "exit_code": command.get("exit_code"),
                        "output_summary": command.get("output_summary").map(truncate_json_text),
                    })
                })
                .collect(),
        ),
        "files_read" | "files_changed" => serde_json::Value::Array(
            value
                .as_array()
                .into_iter()
                .flatten()
                .map(|file| {
                    serde_json::json!({
                        "path": file.get("path"),
                        "purpose": file.get("purpose").map(truncate_json_text),
                    })
                })
                .collect(),
        ),
        "task_coverage" => serde_json::Value::Array(
            value
                .as_array()
                .into_iter()
                .flatten()
                .map(|coverage| {
                    serde_json::json!({
                        "task_id": coverage.get("task_id"),
                        "status": coverage.get("status"),
                        "summary": coverage.get("summary").map(truncate_json_text),
                        "evidence_count": coverage.get("evidence").and_then(serde_json::Value::as_array).map(Vec::len).unwrap_or(0),
                    })
                })
                .collect(),
        ),
        "residual_gaps" => serde_json::Value::Array(
            value
                .as_array()
                .into_iter()
                .flatten()
                .map(|gap| {
                    serde_json::json!({
                        "id": gap.get("id"),
                        "severity": gap.get("severity"),
                        "description": gap.get("description").map(truncate_json_text),
                    })
                })
                .collect(),
        ),
        _ => value.clone(),
    }
}

pub(super) fn compact_unknown_source_value(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::String(_) => truncate_json_text(value),
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(compact_unknown_source_value).collect())
        }
        serde_json::Value::Object(object) => {
            let mut packed = serde_json::Map::new();
            for (key, value) in object {
                packed.insert(key.clone(), compact_unknown_source_value(value));
            }
            serde_json::Value::Object(packed)
        }
        other => other.clone(),
    }
}

pub(super) fn is_large_text_field(key: &str, value: &serde_json::Value) -> bool {
    matches!(
        key,
        "summary" | "output_summary" | "description" | "content"
    ) || value
        .as_str()
        .is_some_and(|text| text.len() > SOURCE_PACK_TEXT_LIMIT)
}

pub(super) fn truncate_json_text(value: &serde_json::Value) -> serde_json::Value {
    let Some(text) = value.as_str() else {
        return value.clone();
    };
    serde_json::Value::String(truncate_for_result(text, SOURCE_PACK_TEXT_LIMIT))
}

pub(super) fn json_array_len(value: &serde_json::Value) -> serde_json::Value {
    serde_json::json!(value.as_array().map(Vec::len).unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packing_strips_tool_declarations_from_reduce_source() {
        // allowed_mcp_tools scans the whole reduce stage input; packed prior
        // agent output must not carry a tool declaration (even nested) or the
        // reducer would bind MCP tools it was never authorised for.
        let packed = source_pack_value(&serde_json::json!({
            "status": "accepted",
            "summary": "prior output",
            "evidence": { "required_tools": ["pine_compile"] },
            "notes": [{ "mcp_tools": ["tv_health_check"] }]
        }));
        let blob = packed.to_string();
        assert!(!blob.contains("required_tools"), "{blob}");
        assert!(!blob.contains("mcp_tools"), "{blob}");
        assert!(!blob.contains("pine_compile"), "{blob}");
    }
}
