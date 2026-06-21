use std::fs;
use std::path::Path;

use archon_workflow::WorkflowStore;
use serde_json::Value;

pub(crate) fn evidence_blocks(store: &WorkflowStore, run_id: &str) -> String {
    let mut out = String::new();
    append_coverage_block(&mut out, store, run_id);
    append_command_block(&mut out, store, run_id);
    append_write_coordination_event_block(&mut out, store, run_id);
    out
}

fn append_coverage_block(out: &mut String, store: &WorkflowStore, run_id: &str) {
    let Ok(run) = store.load_state(run_id) else {
        return;
    };
    let mut rows = Vec::new();
    for stage in run.stages.values() {
        for artifact in &stage.artifacts {
            let path = artifact.path.to_string_lossy();
            if !path.contains("work_unit_coverage") {
                continue;
            }
            let full_path = store.run_dir(run_id).join(&artifact.path);
            let Ok(value) = read_json(&full_path) else {
                continue;
            };
            rows.push(format!(
                "coverage: stage={} verdict={} required={} satisfied={} missing={} blocked={} path={}",
                stage.id,
                field(&value, "verdict"),
                array_len(&value, "required_work_units"),
                array_len(&value, "satisfied_work_units"),
                array_values(&value, "missing_work_units"),
                array_values(&value, "blocked_work_units"),
                artifact.path.display()
            ));
        }
    }
    if !rows.is_empty() {
        out.push_str(&rows.join("\n"));
        out.push('\n');
    }
}

fn append_command_block(out: &mut String, store: &WorkflowStore, run_id: &str) {
    let root = store.run_dir(run_id).join("command-executions");
    let mut rows = command_record_paths(&root)
        .into_iter()
        .filter_map(|path| read_json(&path).ok().map(|value| (path, value)))
        .map(|(path, value)| {
            format!(
                "command: stage={} status={} role={} progress={} exit={} record={}",
                field(&value, "stage_id"),
                field(&value, "status"),
                field(&value, "role"),
                field(&value, "progress_class"),
                value
                    .get("exit_status")
                    .map(Value::to_string)
                    .unwrap_or_else(|| "null".into()),
                path.strip_prefix(store.run_dir(run_id))
                    .unwrap_or(path.as_path())
                    .display()
            )
        })
        .collect::<Vec<_>>();
    rows.sort();
    if !rows.is_empty() {
        out.push_str(&rows.join("\n"));
        out.push('\n');
    }
}

fn append_write_coordination_event_block(out: &mut String, store: &WorkflowStore, run_id: &str) {
    let Ok(raw) = fs::read_to_string(store.events_path(run_id)) else {
        return;
    };
    let mut rows = raw
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|value| {
            value
                .get("kind")
                .and_then(Value::as_str)
                .is_some_and(|kind| kind == "write_coordination_serial_fallback")
        })
        .map(|value| {
            let detail = value.get("detail").unwrap_or(&Value::Null);
            format!(
                "write_coordination: serial_fallback stage={} reason={}",
                field(detail, "stage_id"),
                field(detail, "fallback")
            )
        })
        .collect::<Vec<_>>();
    rows.sort();
    rows.dedup();
    if !rows.is_empty() {
        out.push_str(&rows.join("\n"));
        out.push('\n');
    }
}

fn command_record_paths(root: &Path) -> Vec<std::path::PathBuf> {
    let Ok(stages) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut paths = Vec::new();
    for stage in stages.flatten().filter(|entry| entry.path().is_dir()) {
        let Ok(records) = fs::read_dir(stage.path()) else {
            continue;
        };
        paths.extend(
            records
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json")),
        );
    }
    paths
}

fn read_json(path: &Path) -> Result<Value, serde_json::Error> {
    let raw = fs::read_to_string(path).unwrap_or_default();
    serde_json::from_str(&raw)
}

fn field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| "unknown".into())
}

fn array_len(value: &Value, key: &str) -> usize {
    value.get(key).and_then(Value::as_array).map_or(0, Vec::len)
}

fn array_values(value: &Value, key: &str) -> String {
    let values = value
        .get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    if values.is_empty() {
        "[]".into()
    } else {
        format!("[{}]", values.join(","))
    }
}
