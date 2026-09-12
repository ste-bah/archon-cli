//! Host read orientation survives even when a timed-out workspace is clean.
//! JSONL is written by the tool guard, outside that disposable workspace.
use super::{WorkflowV2Evidence, WorkflowV2EvidenceKind, WorkflowV2Result, WorkflowV2ResultStore};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

const KEY: &str = "workflow_read_set";
const MAX_RANGES: usize = 200;

pub fn path(store: &WorkflowV2ResultStore, call_id: &str) -> PathBuf {
    store
        .root()
        .join("read-sets")
        .join(format!("{:x}.jsonl", Sha256::digest(call_id.as_bytes())))
}

fn load(store: &WorkflowV2ResultStore, call_id: &str) -> Vec<Value> {
    let path = path(store, call_id);
    let file = match std::fs::File::open(&path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(e) => {
            eprintln!("cannot load workflow read set {}: {e}", path.display());
            return Vec::new();
        }
    };
    let mut ranges = BTreeMap::new();
    for line in BufReader::new(file).lines() {
        let record = match line
            .ok()
            .and_then(|line| serde_json::from_str::<Value>(&line).ok())
        {
            Some(record) => record,
            None => continue, // interrupted final append does not erase earlier evidence
        };
        let Some(key) = range_key(&record) else {
            continue;
        };
        if ranges.len() < MAX_RANGES || ranges.contains_key(&key) {
            ranges.insert(key, record);
        }
    }
    ranges.into_values().collect()
}

fn range_key(record: &Value) -> Option<(String, u64, u64)> {
    Some((
        record.get("path")?.as_str()?.to_string(),
        record.get("offset")?.as_u64()?,
        record.get("limit")?.as_u64()?,
    ))
}

fn descriptions(records: &[Value]) -> String {
    records
        .iter()
        .filter_map(range_key)
        .map(|(path, offset, limit)| format!("{path} (offset={offset}, limit={limit}, 0-based)"))
        .collect::<Vec<_>>()
        .join("; ")
}

/// Called after validation (which may replace the result) and before outcome
/// persistence. Independent of patch capture: zero edits still yields evidence.
pub fn attach(store: &WorkflowV2ResultStore, call_id: &str, result: &mut WorkflowV2Result) {
    let records = load(store, call_id);
    if records.is_empty() {
        return;
    }
    if !result.data.is_object() {
        result.data = serde_json::json!({});
    }
    result.data[KEY] = Value::Array(records.clone());
    let mut evidence = WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Review,
        format!(
            "Read-set orientation retained (up to {MAX_RANGES} ranges; not proof of completed work): {}",
            descriptions(&records)
        ),
    );
    evidence.source = Some(path(store, call_id).display().to_string());
    result.evidence.push(evidence);
}

pub fn with_retry_preamble(
    task: &str,
    store: &WorkflowV2ResultStore,
    task_ids: &[String],
) -> String {
    if task_ids.is_empty() {
        return task.to_string();
    }
    let mut ranges = BTreeMap::new();
    for outcome in store.load_branch_outcomes().unwrap_or_default() {
        let Some(result) = outcome.result else {
            continue;
        };
        let matches = result
            .data
            .get("canonical_task_ids")
            .and_then(Value::as_array)
            .is_some_and(|ids| {
                ids.iter()
                    .filter_map(Value::as_str)
                    .any(|id| task_ids.iter().any(|target| target == id))
            });
        if !matches {
            continue;
        }
        for record in result
            .data
            .get(KEY)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(key) = range_key(record)
                && (ranges.len() < MAX_RANGES || ranges.contains_key(&key))
            {
                ranges.insert(key, record.clone());
            }
        }
    }
    if ranges.is_empty() {
        return task.to_string();
    }
    let records: Vec<_> = ranges.into_values().collect();
    format!(
        "Prior attempt read-set orientation (even if it left no patch): {}. These are historical path/range references, not current content or evidence of implementation. Reuse this map rather than rediscovering the tree; refresh only needed ranges. An unchanged Read can be retrieved with force_refresh=true after compaction, within the read budget.\n\n{task}",
        descriptions(&records)
    )
}

/// Immediate branch redispatch happens before any wave outcome is saved.
/// Read the live sidecar, rather than requiring a saved previous result.
pub fn with_current_preamble(task: &str, store: &WorkflowV2ResultStore, call_id: &str) -> String {
    let records = load(store, call_id);
    if records.is_empty() {
        return task.to_string();
    }
    format!(
        "Current attempt read-set orientation: {}. Historical reads are not implementation evidence. Reuse these ranges; use force_refresh=true if prior content was compacted away.\n\n{task}",
        descriptions(&records)
    )
}
