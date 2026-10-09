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
    let mut root_spellings = vec![root.to_path_buf()];
    root_spellings.extend(fs::canonicalize(root));
    let mut root_spellings = root_spellings
        .into_iter()
        .filter_map(|path| path.to_str().map(str::to_owned))
        .flat_map(|spelling| {
            let forward = spelling.replace('\\', "/");
            let backward = spelling.replace('/', "\\");
            [spelling, forward, backward]
        })
        .collect::<Vec<_>>();
    root_spellings.sort_by_key(|spelling| std::cmp::Reverse(spelling.len()));
    root_spellings.dedup();
    let mut replacements = root_spellings
        .iter()
        .map(|spelling| (spelling.as_str(), "<project-root>"))
        .collect::<Vec<_>>();
    replacements.extend([
        ("<project-root>\\", "<project-root>/"),
        (endpoint, "<provider-endpoint>"),
        (run_id, "<run-id>"),
    ]);
    // Receipts are read through their raw destination paths, so they are
    // normalised before the host call ids inside those paths are replaced.
    let (content_by_digest, compared_paths) =
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
        normalize_run_specific_with_receipts(value, &content_by_digest, &compared_paths);
    }
    records.sort_by_key(|record| serde_json::to_string(&record.1).unwrap());
    records
}

pub fn normalize_publication_receipts(
    records: &mut [(String, serde_json::Value)],
    replacements: &[(&str, &str)],
) -> (
    std::collections::HashMap<String, serde_json::Value>,
    std::collections::HashSet<String>,
) {
    let mut content_by_digest = std::collections::HashMap::new();
    let mut compared_paths = std::collections::HashSet::new();
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
            content_by_digest.insert(digest_normalized(&content), content.clone());
            content_by_digest.insert(digest.to_owned(), content);
            compared_paths.insert(path.to_owned());
        }
    }

    for (_, record) in records.iter_mut() {
        if let Some(entries) = record
            .pointer_mut("/result/data/publicationReceipt/entries")
            .and_then(serde_json::Value::as_array_mut)
        {
            for entry in entries {
                let path = entry
                    .get("destination_path")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
                    .unwrap_or_else(|| {
                        panic!("publication receipt entry is missing destination_path")
                    });
                let bytes = fs::read(&path).unwrap_or_else(|error| {
                    panic!("reading published receipt file {path}: {error}")
                });
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
    (content_by_digest, compared_paths)
}

fn normalize_derived_hashes(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            // The build's own commit is stamped into pins and gates; it changes
            // with every commit of the code under test, not with the run.
            if map.contains_key("binary_commit") {
                map.insert("binary_commit".into(), "<binary-commit>".into());
            }
            // The preimages are this record's own call input and result, which
            // the snapshot compares after normalisation, so these run-specific
            // hashes are represented by placeholders.
            // Copies of a record nested in a later call's inputs carry the
            // same hashes without every top-level key, so a record is any
            // object holding the hashes beside its call or result.
            let is_call_record = (map.contains_key("input_hash")
                || map.contains_key("output_hash"))
                && (map.contains_key("call") || map.contains_key("result"));
            let normalized_input = is_call_record
                .then(|| map.get("call"))
                .flatten()
                .map(|_| "<normalized-call-input>".to_owned());
            let normalized_output = is_call_record
                .then(|| map.get("result"))
                .flatten()
                .map(|_| "<normalized-call-result>".to_owned());
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
    normalize_run_specific_with_receipts(value, &Default::default(), &Default::default());
}

fn normalize_run_specific_with_receipts(
    value: &mut serde_json::Value,
    content_by_digest: &std::collections::HashMap<String, serde_json::Value>,
    compared_paths: &std::collections::HashSet<String>,
) {
    strip_timestamps(value);
    normalize_derived_hashes(value);
    normalize_receipt_hashes(value, content_by_digest, compared_paths);
    match value {
        serde_json::Value::String(text) => {
            if let Ok(mut embedded @ (serde_json::Value::Object(_) | serde_json::Value::Array(_))) =
                serde_json::from_str::<serde_json::Value>(text)
            {
                normalize_run_specific_with_receipts(
                    &mut embedded,
                    content_by_digest,
                    compared_paths,
                );
                *text = embedded.to_string();
            }
        }
        serde_json::Value::Object(map) => map.values_mut().for_each(|nested| {
            normalize_run_specific_with_receipts(nested, content_by_digest, compared_paths)
        }),
        serde_json::Value::Array(values) => values.iter_mut().for_each(|nested| {
            normalize_run_specific_with_receipts(nested, content_by_digest, compared_paths)
        }),
        _ => {}
    }
}

#[cfg(test)]
fn normalize_embedded_receipt_for_test(
    value: &mut serde_json::Value,
    content_by_digest: &std::collections::HashMap<String, serde_json::Value>,
) {
    normalize_run_specific_with_receipts(value, content_by_digest, &Default::default());
}

fn normalize_receipt_hashes(
    value: &mut serde_json::Value,
    content_by_digest: &std::collections::HashMap<String, serde_json::Value>,
    compared_paths: &std::collections::HashSet<String>,
) {
    match value {
        serde_json::Value::Object(map) => {
            let relative_path = map
                .get("relative_path")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned);
            let is_receipt_entry = relative_path.is_some()
                && (map.contains_key("blake3") || map.contains_key("prior_blake3"));
            if is_receipt_entry {
                let compared_file = relative_path
                    .as_deref()
                    .map(|relative| {
                        let relative = Path::new(relative);
                        compared_paths
                            .iter()
                            .any(|destination| Path::new(destination).ends_with(relative))
                    })
                    .unwrap_or(false);
                for key in ["blake3", "prior_blake3"] {
                    let Some(digest) = map.get(key).and_then(serde_json::Value::as_str) else {
                        continue;
                    };
                    if let Some(content) = content_by_digest.get(digest) {
                        map.insert(key.into(), digest_normalized(content).into());
                    } else if digest.starts_with('<')
                        || content_by_digest
                            .values()
                            .any(|content| digest_normalized(content) == digest)
                    {
                        // Already normalised by an outer pass: keep it.
                    } else {
                        // The bytes behind this digest were overwritten later
                        // (a pin or lock rewritten by a later call) and embed
                        // timestamps, so the digest and length are run-specific.
                        // The final bytes at every destination are compared as
                        // the snapshot's artifacts; `compared_file` says so.
                        let label = if compared_file {
                            "<receipt-digest-with-compared-content>"
                        } else {
                            "<receipt-digest-of-overwritten-content>"
                        };
                        map.insert(key.into(), label.into());
                    }
                }
                if map
                    .get("blake3")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|d| d.starts_with('<'))
                {
                    map.remove("byte_len");
                }
            }
            for nested in map.values_mut() {
                normalize_receipt_hashes(nested, content_by_digest, compared_paths);
            }
        }
        serde_json::Value::Array(values) => values
            .iter_mut()
            .for_each(|nested| normalize_receipt_hashes(nested, content_by_digest, compared_paths)),
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
