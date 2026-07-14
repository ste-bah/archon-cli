use serde_json::Value;

use crate::command::workflow_live::workflow_live_generated_lifecycle_support as support;

pub(super) fn prepare_verification_items(
    mut items: Vec<Value>,
    project_artifact_root: Option<&str>,
    implementation_evidence: &[Value],
) -> Vec<Value> {
    add_substantive_coverage_verification(&mut items, project_artifact_root);
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

fn add_substantive_coverage_verification(
    items: &mut Vec<Value>,
    project_artifact_root: Option<&str>,
) {
    let Some(source_item_id) = items.iter().find_map(|item| {
        support::strings_of(item.get("canonical_task_ids"))
            .iter()
            .any(|task_id| task_id == "TASK-TDL-080")
            .then(|| {
                item.get("source_item_id")
                    .and_then(Value::as_str)
                    .unwrap_or("TASK-TDL-080")
                    .to_string()
            })
    }) else {
        return;
    };
    if items.iter().any(|item| {
        item.get("item_id").and_then(Value::as_str)
            == Some("verify-TASK-TDL-080-required-native-coverage")
    }) {
        return;
    }
    let root = project_artifact_root.unwrap_or(".");
    let root_literal = serde_json::to_string(root).expect("project root JSON string");
    let command = format!(
        r#"python3 - <<'PY'
import json, pathlib, sys
root = pathlib.Path({root_literal})
coverage = json.loads((root / '.archon/trading-lab/data/coverage/latest.json').read_text())
registry = json.loads((root / '.archon/trading-lab/data/registry.json').read_text())
required = {{(instrument, timeframe) for instrument in ['ES','NQ','SPY','QQQ','BTCUSDT','ETHUSDT'] for timeframe in ['1W','1D','240','60','15']}}
cells = {{(cell.get('canonical_instrument'), cell.get('timeframe')): cell for cell in coverage.get('cells', [])}}
failures = []
for key in sorted(required):
    cell = cells.get(key)
    if cell is None:
        failures.append(f'{{key[0]}}:{{key[1]}} missing coverage cell')
        continue
    required_flags = cell.get('available') is True and cell.get('native_interval') is True and cell.get('production_eligible') is True
    dataset_id, version = cell.get('dataset_id'), cell.get('version')
    if not required_flags or not dataset_id or not version or int(cell.get('row_count') or 0) <= 0:
        failures.append(f'{{key[0]}}:{{key[1]}} unavailable/non-native/non-production/empty')
        continue
    record = registry.get('datasets', {{}}).get(f'{{dataset_id}}:{{version}}')
    if not record or record.get('native_interval') is not True or record.get('production_eligible') is not True or record.get('status') != 'Healthy' or int(record.get('bars') or 0) <= 0:
        failures.append(f'{{key[0]}}:{{key[1]}} lacks healthy registered native provenance')
extra = sorted(set(cells) - required)
if failures or extra or coverage.get('gaps'):
    print(json.dumps({{'failures': failures, 'extra_cells': extra, 'gap_count': len(coverage.get('gaps', []))}}, indent=2))
    sys.exit(1)
print(json.dumps({{'required_cells': len(required), 'registered_native_cells': len(required), 'status': 'production_eligible'}}, indent=2))
PY"#
    );
    items.push(serde_json::json!({
        "item_id": "verify-TASK-TDL-080-required-native-coverage",
        "source_item_id": source_item_id,
        "canonical_task_ids": ["TASK-TDL-080"],
        "focused_verification": command,
        "expected_evidence": "All 30 required universe cells are present, non-empty, native, production eligible, and resolve to Healthy registry records; gaps is empty; redacted provider_env_proof shows ~/.profile was sourced and POLYGON_API_KEY is present.",
        "artifact_requirements": [
            format!("{root}/.archon/trading-lab/data/coverage/latest.json"),
            format!("{root}/.archon/trading-lab/data/registry.json")
        ],
        "provider_env_requirements": ["POLYGON_API_KEY"],
        "profile_sources": ["~/.profile"],
        "required_tools": ["mcp__tradingview__data_get_ohlcv"]
    }));
}

pub(super) fn verification_options(items: &[Value], task: &str, focused: bool) -> Value {
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
