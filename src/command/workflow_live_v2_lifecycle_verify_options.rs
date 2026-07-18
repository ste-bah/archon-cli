use serde_json::Value;

use crate::command::workflow_live::workflow_live_generated_lifecycle_support as support;

pub(super) fn prepare_verification_items(
    mut items: Vec<Value>,
    project_artifact_root: Option<&str>,
    implementation_evidence: &[Value],
    task_universe: &Value,
) -> Vec<Value> {
    add_declared_deliverable_verifications(&mut items, project_artifact_root, task_universe);
    bind_contract_verifiers_to_cited_artifacts(&mut items, project_artifact_root, task_universe);
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

const MAX_BOUND_CONTRACT_VERIFICATIONS: usize = 4;

/// D81: evidence must pass CURRENT gates at cite time, never by mere presence.
/// When a plan item cites a concrete artifact instance whose path matches a
/// declared deliverable-contract template, the host binds that contract's
/// verification command to the item — the cited instance faces the typed
/// verifier and substance/registry predicates regardless of what the plan
/// chose to check on its own.
fn bind_contract_verifiers_to_cited_artifacts(
    items: &mut [Value],
    project_artifact_root: Option<&str>,
    task_universe: &Value,
) {
    let root = project_artifact_root.unwrap_or(".");
    let contracts: Vec<(String, Value)> = support::array(task_universe.get("tasks"))
        .iter()
        .flat_map(|task| {
            let task_id = task
                .get("canonical_task_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            support::array(task.get("deliverable_contracts"))
                .into_iter()
                .filter(|contract| {
                    contract
                        .get("artifact_path")
                        .and_then(Value::as_str)
                        .is_some()
                })
                .map(move |contract| (task_id.clone(), contract))
        })
        .collect();
    if contracts.is_empty() {
        return;
    }
    for item in items.iter_mut() {
        if item.get("deliverable_contract").is_some() {
            continue;
        }
        let item_task_ids = support::strings_of(item.get("canonical_task_ids"));
        let cited = cited_artifact_paths(item.get("artifact_requirements"));
        let mut bound_commands = std::collections::BTreeSet::new();
        for path in &cited {
            for (task_id, contract) in &contracts {
                if !item_task_ids.contains(task_id) {
                    continue;
                }
                let template = contract
                    .get("artifact_path")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if !templated_path_matches(template, path, root) {
                    continue;
                }
                let mut bound = contract.clone();
                bound["artifact_path"] = Value::String(path.clone());
                bound_commands.insert(
                    super::workflow_live_v2_deliverable_contract::verification_command(
                        root, &bound,
                    ),
                );
                if bound_commands.len() >= MAX_BOUND_CONTRACT_VERIFICATIONS {
                    break;
                }
            }
            if bound_commands.len() >= MAX_BOUND_CONTRACT_VERIFICATIONS {
                break;
            }
        }
        if bound_commands.is_empty() {
            continue;
        }
        let Some(object) = item.as_object_mut() else {
            continue;
        };
        let mut commands = match object.remove("focused_verification") {
            Some(Value::Array(values)) => values,
            Some(Value::String(text)) if !text.trim().is_empty() => {
                vec![Value::String(text)]
            }
            _ => Vec::new(),
        };
        commands.extend(bound_commands.into_iter().map(Value::String));
        object.insert("focused_verification".to_string(), Value::Array(commands));
        object.insert(
            "host_bound_contract_verification".to_string(),
            Value::Bool(true),
        );
    }
}

fn cited_artifact_paths(value: Option<&Value>) -> Vec<String> {
    fn collect(value: &Value, paths: &mut Vec<String>) {
        match value {
            Value::String(path) if !path.trim().is_empty() => paths.push(path.trim().to_string()),
            Value::Array(values) => values.iter().for_each(|value| collect(value, paths)),
            Value::Object(object) => {
                for key in ["path", "artifact_path", "artifactPath"] {
                    if let Some(Value::String(path)) = object.get(key)
                        && !path.trim().is_empty()
                    {
                        paths.push(path.trim().to_string());
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    let mut paths = Vec::new();
    if let Some(value) = value {
        collect(value, &mut paths);
    }
    paths
}

/// Segment-wise template match: a `<placeholder>` segment matches any single
/// non-empty path segment; literal segments must match exactly. The cited
/// path may be absolute — it is relativized against the artifact root first.
fn templated_path_matches(template: &str, cited: &str, root: &str) -> bool {
    let cited_path = std::path::Path::new(cited);
    let cited = if cited_path.is_absolute() {
        let Ok(relative) = cited_path.strip_prefix(std::path::Path::new(root)) else {
            return false;
        };
        relative.to_string_lossy()
    } else {
        cited_path.to_string_lossy()
    };
    let template_segments: Vec<&str> = template.split('/').filter(|s| !s.is_empty()).collect();
    let cited_segments: Vec<&str> = cited.split('/').filter(|s| !s.is_empty()).collect();
    if template_segments.is_empty() || template_segments.len() != cited_segments.len() {
        return false;
    }
    template_segments
        .iter()
        .zip(&cited_segments)
        .all(|(template_segment, cited_segment)| {
            if template_segment.starts_with('<')
                && template_segment.ends_with('>')
                && template_segment.len() > 2
                && !template_segment[1..template_segment.len() - 1]
                    .chars()
                    .any(|ch| matches!(ch, '<' | '>'))
            {
                !cited_segment.is_empty()
            } else {
                template_segment == cited_segment
            }
        })
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

#[cfg(test)]
mod cited_artifact_binding_tests {
    use super::*;

    fn universe_with_contract() -> Value {
        serde_json::json!({
            "tasks": [
                {
                    "canonical_task_id": "TASK-EX-001",
                    "deliverable_contracts": [{
                        "kind": "instance_manifest",
                        "artifact_path": ".archon/lab-data/sets/<set-id>/<version>/manifest.json",
                        "typed_verifier_command": "verify-tool check {artifact_path}",
                    }],
                },
                {
                    "canonical_task_id": "TASK-OTHER-001",
                    "deliverable_contracts": [{
                        "kind": "other_manifest",
                        "artifact_path": ".archon/lab-data/sets/<set-id>/<version>/manifest.json",
                        "typed_verifier_command": "other-tool check {artifact_path}",
                    }],
                }
            ]
        })
    }

    #[test]
    fn cited_instance_matching_contract_template_gets_bound_verifier() {
        let items = vec![serde_json::json!({
            "item_id": "verify-plan-item-1",
            "canonical_task_ids": ["TASK-EX-001"],
            "artifact_requirements": [{
                "path": "/proj/.archon/lab-data/sets/alpha/v1/manifest.json"
            }],
            "focused_verification": "parse the manifest fields",
        })];
        let prepared =
            prepare_verification_items(items, Some("/proj"), &[], &universe_with_contract());
        let item = prepared
            .iter()
            .find(|item| item["item_id"] == "verify-plan-item-1")
            .expect("plan item");
        assert_eq!(item["host_bound_contract_verification"], true);
        let commands = item["focused_verification"].as_array().expect("array");
        assert!(commands.len() >= 2, "original + bound command");
        let joined = commands
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("parse the manifest fields"));
        assert!(joined.contains("verify-tool check"));
        assert!(!joined.contains("other-tool check"));
        assert!(joined.contains("/proj/.archon/lab-data/sets/alpha/v1/manifest.json"));
    }

    #[test]
    fn unmatched_citation_and_synthesized_items_are_untouched() {
        let items = vec![serde_json::json!({
            "item_id": "verify-plan-item-2",
            "canonical_task_ids": ["TASK-EX-001"],
            "artifact_requirements": ["/proj/other/report.txt"],
            "focused_verification": "read the report",
        })];
        let prepared =
            prepare_verification_items(items, Some("/proj"), &[], &universe_with_contract());
        let item = prepared
            .iter()
            .find(|item| item["item_id"] == "verify-plan-item-2")
            .expect("plan item");
        assert!(item.get("host_bound_contract_verification").is_none());
        assert_eq!(item["focused_verification"], "read the report");
        // The synthesized contract item carries its own verifier and is not re-bound.
        let synthesized = prepared
            .iter()
            .find(|item| item["item_id"] == "verify-TASK-EX-001-instance-manifest")
            .expect("synthesized contract item");
        assert!(synthesized.get("deliverable_contract").is_some());
    }

    #[test]
    fn template_matching_is_segment_wise() {
        assert!(templated_path_matches(
            ".archon/lab-data/sets/<set-id>/<version>/manifest.json",
            ".archon/lab-data/sets/alpha/v2/manifest.json",
            "/proj"
        ));
        assert!(!templated_path_matches(
            ".archon/lab-data/sets/<set-id>/<version>/manifest.json",
            ".archon/lab-data/sets/alpha/manifest.json",
            "/proj"
        ));
        assert!(!templated_path_matches(
            ".archon/lab-data/sets/<set-id>/<version>/manifest.json",
            ".archon/other/sets/alpha/v2/manifest.json",
            "/proj"
        ));
        assert!(!templated_path_matches(
            ".archon/lab-data/sets/<set-id>-suffix/<version>/manifest.json",
            ".archon/lab-data/sets/alpha/v2/manifest.json",
            "/proj"
        ));
        assert!(!templated_path_matches(
            ".archon/lab-data/sets/<set-id>/<version>/manifest.json",
            "/project-other/.archon/lab-data/sets/alpha/v2/manifest.json",
            "/proj"
        ));
    }
}
