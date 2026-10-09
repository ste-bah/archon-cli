use std::fs;
use std::path::{Path, PathBuf};

pub fn golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/offline_decomposition_golden.json")
}

pub fn assert_golden_snapshot(actual: &serde_json::Value) {
    let path = golden_path();
    if std::env::var_os("ARCHON_UPDATE_GOLDEN").as_deref() == Some(std::ffi::OsStr::new("1")) {
        fs::write(&path, serde_json::to_vec_pretty(actual).unwrap()).unwrap();
        return;
    }
    assert_eq!(
        actual,
        &read_golden_snapshot(),
        "fresh run differs from fixed golden snapshot"
    );
}

pub fn read_golden_snapshot() -> serde_json::Value {
    let path = golden_path();
    serde_json::from_slice(&fs::read(&path).unwrap_or_else(|error| {
        panic!(
            "reading fixed lifecycle golden {}: {error}; generate it with ARCHON_UPDATE_GOLDEN=1",
            path.display()
        )
    }))
    .unwrap_or_else(|error| panic!("parsing fixed lifecycle golden {}: {error}", path.display()))
}

pub fn strip_timestamps(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            map.retain(|key, _| {
                !(key == "timestamp"
                    || key == "evaluated_at"
                    || key.ends_with("_at")
                    || key.ends_with("At"))
            });
            for nested in map.values_mut() {
                strip_timestamps(nested);
            }
        }
        serde_json::Value::Array(values) => {
            for nested in values {
                strip_timestamps(nested);
            }
        }
        _ => {}
    }
}

pub fn normalize_values(value: &mut serde_json::Value, replacements: &[(&str, &str)]) {
    match value {
        serde_json::Value::String(text) => {
            for (from, to) in replacements {
                *text = text.replace(from, to);
            }
        }
        serde_json::Value::Object(map) => {
            for nested in map.values_mut() {
                normalize_values(nested, replacements);
            }
        }
        serde_json::Value::Array(values) => {
            for nested in values {
                normalize_values(nested, replacements);
            }
        }
        _ => {}
    }
}

pub fn result_store_records(
    run: &Path,
    root: &Path,
    endpoint: &str,
    run_id: &str,
) -> Vec<(String, serde_json::Value)> {
    let results = run.join("v2/results");
    let mut records = fs::read_dir(&results)
        .unwrap_or_else(|error| {
            panic!(
                "reading result-store records at {}: {error}",
                results.display()
            )
        })
        .map(|entry| {
            let path = entry.unwrap().path();
            if !path.is_file()
                || path.extension().and_then(|extension| extension.to_str()) != Some("json")
            {
                return None;
            }
            let mut value: serde_json::Value =
                serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            strip_timestamps(&mut value);
            // Result filenames are hashes of call and attempt data. Compare the
            // retained records by their normalized contents, not those opaque IDs.
            Some((String::new(), value))
        })
        .flatten()
        .collect::<Vec<_>>();
    let replacements = [
        (root.to_str().unwrap(), "<project-root>"),
        (endpoint, "<provider-endpoint>"),
        (run_id, "<run-id>"),
    ];
    // Receipts are read through their raw destination paths, so they are
    // normalised before the host call ids inside those paths are replaced.
    normalize_publication_receipts(&mut records, &replacements);
    // A host call id is a digest of its inputs, so it differs per run; later
    // records carry earlier ids in their inputs, receipts and stdout.
    let host_call_ids: Vec<String> = records
        .iter()
        .filter(|(_, value)| {
            value
                .pointer("/call/method")
                .and_then(serde_json::Value::as_str)
                == Some("hostCommand")
        })
        .filter_map(|(_, value)| {
            value
                .pointer("/call/id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
        .collect();
    let host_call_replacements: Vec<(&str, &str)> = host_call_ids
        .iter()
        .map(|id| (id.as_str(), "<host-call-id>"))
        .collect();
    for (_, value) in &mut records {
        normalize_values(value, &host_call_replacements);
        normalize_lifecycle_metadata(value);
        normalize_values(value, &replacements);
        normalize_run_specific(value);
    }
    records.sort_by_key(|record| serde_json::to_string(&record.1).unwrap());
    records
}

pub fn normalize_publication_receipts(
    records: &mut [(String, serde_json::Value)],
    replacements: &[(&str, &str)],
) {
    let mut content_by_digest = std::collections::HashMap::new();
    for (_, record) in records.iter() {
        let Some(entries) = record
            .pointer("/result/data/publicationReceipt/entries")
            .and_then(serde_json::Value::as_array)
        else {
            continue;
        };
        for entry in entries {
            let (Some(path), Some(digest)) = (
                entry
                    .get("destination_path")
                    .and_then(serde_json::Value::as_str),
                entry.get("blake3").and_then(serde_json::Value::as_str),
            ) else {
                continue;
            };
            let bytes = fs::read(path)
                .unwrap_or_else(|error| panic!("reading published receipt file {path}: {error}"));
            if archon_workflow::task_set_contract::content_digest(&bytes) != digest {
                continue;
            }
            let content = normalized_published_content(&bytes, replacements);
            content_by_digest.insert(digest.to_owned(), content);
        }
    }

    for (_, record) in records.iter_mut() {
        let Some(entries) = record
            .pointer_mut("/result/data/publicationReceipt/entries")
            .and_then(serde_json::Value::as_array_mut)
        else {
            continue;
        };
        for entry in entries {
            let path = entry
                .get("destination_path")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| panic!("publication receipt entry is missing destination_path"));
            let bytes = fs::read(&path)
                .unwrap_or_else(|error| panic!("reading published receipt file {path}: {error}"));
            let content = normalized_published_content(&bytes, replacements);
            if let Some(map) = entry.as_object_mut() {
                let prior_content = map
                    .get("prior_blake3")
                    .and_then(serde_json::Value::as_str)
                    .and_then(|digest| content_by_digest.get(digest).cloned());
                if let Some(prior_content) = prior_content {
                    // The prior digest is replaceable only because its earlier
                    // published preimage is also present in this snapshot.
                    map.insert("prior_content".into(), prior_content.clone());
                    map.insert(
                        "prior_blake3".into(),
                        digest_normalized(&prior_content).into(),
                    );
                }
                map.remove("byte_len");
                map.insert("blake3".into(), digest_normalized(&content).into());
                map.insert("content".into(), content);
            }
        }
    }
}

fn normalize_derived_hashes(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            // Call hashes are normalized only when their exact preimages are
            // compared too: this record's call input and result are retained
            // in the snapshot, so hash their normalized content rather than
            // erasing the digest. Other digest-looking fields stay raw here.
            let is_call_record = ["call", "attempt", "input_hash", "status", "result"]
                .iter()
                .all(|key| map.contains_key(*key));
            let normalized_input = is_call_record
                .then(|| map.get("call"))
                .flatten()
                .map(|call| digest_normalized(call.get("input").unwrap_or(call)));
            let normalized_output = is_call_record
                .then(|| map.get("result"))
                .flatten()
                .map(digest_normalized);
            if is_call_record
                && map.contains_key("input_hash")
                && let Some(digest) = normalized_input
            {
                map.insert("input_hash".into(), digest.into());
            }
            if is_call_record
                && map.contains_key("output_hash")
                && let Some(digest) = normalized_output
            {
                map.insert("output_hash".into(), digest.into());
            }
            for nested in map.values_mut() {
                normalize_derived_hashes(nested);
            }
        }
        serde_json::Value::Array(values) => {
            for nested in values {
                normalize_derived_hashes(nested);
            }
        }
        serde_json::Value::String(path) => {
            const PIN_DIRECTORY: &str = ".archon/task-set-pins/";
            if let Some(start) = path.find(PIN_DIRECTORY) {
                let filename_start = start + PIN_DIRECTORY.len();
                let filename_end = path[filename_start..]
                    .find('/')
                    .map(|offset| filename_start + offset)
                    .unwrap_or(path.len());
                let filename = &path[filename_start..filename_end];
                if filename.ends_with(".json") {
                    path.replace_range(filename_start..filename_end, "<pin>.json");
                }
            }
        }
        _ => {}
    }
}

fn digest_normalized(value: &serde_json::Value) -> String {
    archon_workflow::task_set_contract::content_digest(&serde_json::to_vec(value).unwrap())
}

/// Strip timestamps in records and JSON carried as a string (stdout, embedded
/// receipts); preserve every digest unless the compared preimage is in-snapshot.
fn normalize_run_specific(value: &mut serde_json::Value) {
    strip_timestamps(value);
    normalize_derived_hashes(value);
    match value {
        serde_json::Value::String(text) => {
            if let Ok(mut embedded @ (serde_json::Value::Object(_) | serde_json::Value::Array(_))) =
                serde_json::from_str::<serde_json::Value>(text)
            {
                normalize_run_specific(&mut embedded);
                *text = embedded.to_string();
            }
        }
        serde_json::Value::Object(map) => map.values_mut().for_each(normalize_run_specific),
        serde_json::Value::Array(values) => values.iter_mut().for_each(normalize_run_specific),
        _ => {}
    }
}

fn normalized_published_content(bytes: &[u8], replacements: &[(&str, &str)]) -> serde_json::Value {
    match serde_json::from_slice(bytes) {
        Ok(mut value) => {
            normalize_values(&mut value, replacements);
            normalize_run_specific(&mut value);
            value
        }
        Err(_) => {
            let mut value = serde_json::Value::String(String::from_utf8_lossy(bytes).into_owned());
            normalize_values(&mut value, replacements);
            value
        }
    }
}

#[cfg(test)]
#[path = "workflow_decomposition_offline_snapshot_tests.rs"]
mod tests;

fn normalize_lifecycle_metadata(value: &mut serde_json::Value) {
    let host_call_id = value
        .get("call")
        .and_then(|call| {
            (call.get("method").and_then(serde_json::Value::as_str) == Some("hostCommand"))
                .then(|| call.get("id").and_then(serde_json::Value::as_str))
                .flatten()
        })
        .map(str::to_owned);
    if let Some(host_call_id) = host_call_id {
        replace_string_values(value, &host_call_id, "<host-call-id>");
        if let Some(call) = value
            .get_mut("call")
            .and_then(serde_json::Value::as_object_mut)
        {
            call.insert(
                "id".into(),
                serde_json::Value::String("<host-call-id>".into()),
            );
        }
    }

    match value {
        serde_json::Value::Object(map) => {
            // Resume adds an admission event and retries only the interrupted
            // skeleton call, so these values describe this run's execution path.
            map.remove("admission_sequence");
            if map.get("call").and_then(|call| call.get("id"))
                == Some(&serde_json::Value::String("skeleton-author-1".into()))
            {
                map.remove("attempt");
            }
            for (key, nested) in map.iter_mut() {
                if key == "call_id" {
                    *nested = serde_json::Value::String("<call-id>".into());
                } else {
                    normalize_lifecycle_metadata(nested);
                }
            }
        }
        serde_json::Value::Array(values) => {
            for nested in values {
                normalize_lifecycle_metadata(nested);
            }
        }
        _ => {}
    }
}

fn replace_string_values(value: &mut serde_json::Value, from: &str, to: &str) {
    match value {
        serde_json::Value::String(text) => *text = text.replace(from, to),
        serde_json::Value::Object(map) => {
            for nested in map.values_mut() {
                replace_string_values(nested, from, to);
            }
        }
        serde_json::Value::Array(values) => {
            for nested in values {
                replace_string_values(nested, from, to);
            }
        }
        _ => {}
    }
}
