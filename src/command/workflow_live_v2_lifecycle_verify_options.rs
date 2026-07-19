use serde_json::Value;

use crate::command::workflow_live::workflow_live_generated_lifecycle_support as support;

pub(super) fn prepare_verification_items(
    mut items: Vec<Value>,
    project_artifact_root: Option<&str>,
    implementation_evidence: &[Value],
    task_universe: &Value,
) -> Vec<Value> {
    add_declared_deliverable_verifications(&mut items, project_artifact_root, task_universe);
    let scopes =
        super::workflow_live_v2_lifecycle_verify_scope::manifest_scopes(implementation_evidence);
    items
        .into_iter()
        .map(|mut item| {
            if let (Some(root), Some(object)) = (project_artifact_root, item.as_object_mut()) {
                object.insert(
                    "project_artifact_root".to_string(),
                    Value::String(root.to_string()),
                );
            }
            super::workflow_live_v2_lifecycle_verify_scope::stamp_manifest_scope(
                &mut item, &scopes,
            );
            item
        })
        .collect()
}

fn add_declared_deliverable_verifications(
    items: &mut Vec<Value>,
    project_artifact_root: Option<&str>,
    task_universe: &Value,
) {
    let root = project_artifact_root.unwrap_or(".");
    let tasks = support::array(task_universe.get("tasks"));
    for task in tasks {
        let Some(task_id) = task.get("canonical_task_id").and_then(Value::as_str) else {
            continue;
        };
        let Some(source_item_id) = items.iter().find_map(|item| {
            support::strings_of(item.get("canonical_task_ids"))
                .iter()
                .any(|candidate| candidate == task_id)
                .then(|| {
                    item.get("source_item_id")
                        .and_then(Value::as_str)
                        .unwrap_or(task_id)
                        .to_string()
                })
        }) else {
            continue;
        };
        for contract in support::array(task.get("deliverable_contracts")) {
            let Some(kind) = contract.get("kind").and_then(Value::as_str) else {
                continue;
            };
            let Some(artifact_path) = contract.get("artifact_path").and_then(Value::as_str) else {
                continue;
            };
            let item_id = format!(
                "verify-{task_id}-{}",
                kind.chars()
                    .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
                    .collect::<String>()
            );
            if items
                .iter()
                .any(|item| item.get("item_id").and_then(Value::as_str) == Some(&item_id))
            {
                continue;
            }
            let command =
                super::workflow_live_v2_deliverable_contract::verification_command(root, &contract);
            let mut artifact_requirements = vec![artifact_path.to_string()];
            if let Some(registry_path) = contract.get("registry_path").and_then(Value::as_str) {
                artifact_requirements.push(registry_path.to_string());
            }
            items.push(serde_json::json!({
                "item_id": item_id,
                "source_item_id": source_item_id,
                "canonical_task_ids": [task_id],
                "focused_verification": command,
                "expected_evidence": "The declared deliverable is non-empty and its generated contract verification passes every declared identity, field, count, internal-consistency, payload-substance, cross-identity, registry, and gap predicate.",
                "artifact_requirements": artifact_requirements,
                "provider_env_requirements": support::strings_of(task.get("required_env_keys")),
                "required_tools": support::strings_of(task.get("required_tools")),
                "deliverable_contract": contract,
            }));
        }
    }
}

pub(super) fn verification_options(items: &[Value], task: &str, focused: bool) -> Value {
    let task = super::workflow_live_v2_lifecycle_prompts::ground_host_manifest_schema(task);
    let mut options = serde_json::json!({ "tier": "coder", "task": task });
    if focused {
        options["itemKind"] = Value::String("focused_verification".to_string());
    }
    if items_have_cargo_commands(items) {
        options["maxParallelism"] = serde_json::json!(1);
    }
    options
}

pub(super) fn write_wave_parallelism(items: &[Value]) -> Value {
    if items_have_cargo_commands(items) {
        serde_json::json!(1)
    } else {
        Value::String("configured".to_string())
    }
}

fn items_have_cargo_commands(items: &[Value]) -> bool {
    items.iter().any(|item| {
        support::raw_strings(
            item,
            &[
                "focused_verification",
                "commands",
                "command",
                "expected_evidence",
            ],
        )
        .iter()
        .any(|text| text.to_ascii_lowercase().contains("cargo "))
    })
}
