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
    let root_literal = serde_json::to_string(root).expect("project root JSON string");
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
            let contract_literal = serde_json::to_string(&contract).expect("contract JSON");
            let command = format!(
                r#"python3 - <<'PY'
import json, pathlib, sys
root = pathlib.Path({root_literal})
contract = json.loads({contract_literal:?})
def resolve(value):
    path = pathlib.Path(value)
    return path if path.is_absolute() else root / path
artifact_path = resolve(contract['artifact_path'])
if not artifact_path.is_file() or artifact_path.stat().st_size == 0:
    raise SystemExit(f'missing or empty declared deliverable: {{artifact_path}}')
artifact = json.loads(artifact_path.read_text())
registry = None
if contract.get('registry_path'):
    registry_path = resolve(contract['registry_path'])
    if not registry_path.is_file():
        raise SystemExit(f'missing declared registry: {{registry_path}}')
    registry = json.loads(registry_path.read_text())
if not contract.get('required_universe'):
    print(json.dumps({{'status': 'declared_deliverable_present', 'artifact': str(artifact_path)}}))
    raise SystemExit(0)
required = {{(instrument, timeframe) for instrument in artifact.get('instruments', []) for timeframe in artifact.get('timeframes', [])}}
if not required:
    raise SystemExit('declared required universe is empty')
cells = {{(cell.get('canonical_instrument'), cell.get('timeframe')): cell for cell in artifact.get('cells', [])}}
failures = []
for key in sorted(required):
    cell = cells.get(key)
    if cell is None:
        failures.append(f'{{key[0]}}:{{key[1]}} missing coverage cell')
        continue
    symbol = cell.get('symbol') or cell.get('provider_symbol')
    interval = cell.get('interval') or cell.get('timeframe')
    required_flags = cell.get('available') is True and cell.get('native_interval') is True and cell.get('production_eligible') is True and bool(symbol) and bool(interval)
    dataset_id, version = cell.get('dataset_id'), cell.get('version')
    if not required_flags or not dataset_id or not version or int(cell.get('row_count') or 0) <= 0:
        failures.append(f'{{key[0]}}:{{key[1]}} unavailable/non-native/non-production/empty/missing-symbol-or-interval')
        continue
    if registry is not None:
        record = registry.get('datasets', {{}}).get(f'{{dataset_id}}:{{version}}')
        if not record or record.get('native_interval') is not True or record.get('production_eligible') is not True or record.get('status') != 'Healthy' or int(record.get('bars') or 0) <= 0:
            failures.append(f'{{key[0]}}:{{key[1]}} lacks healthy registered native provenance')
extra = sorted(set(cells) - required)
if failures or extra or artifact.get('gaps'):
    print(json.dumps({{'failures': failures, 'extra_cells': extra, 'gap_count': len(artifact.get('gaps', []))}}, indent=2))
    sys.exit(1)
print(json.dumps({{'required_cells': len(required), 'registered_native_cells': len(required), 'status': 'production_eligible'}}, indent=2))
PY"#
            );
            let mut artifact_requirements = vec![artifact_path.to_string()];
            if let Some(registry_path) = contract.get("registry_path").and_then(Value::as_str) {
                artifact_requirements.push(registry_path.to_string());
            }
            items.push(serde_json::json!({
                "item_id": item_id,
                "source_item_id": source_item_id,
                "canonical_task_ids": [task_id],
                "focused_verification": command,
                "expected_evidence": "The declared deliverable is non-empty and every declared required-universe cell is substantive, native, production eligible, gap-free, and registry-backed when a registry is declared.",
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
