use std::fs;
use std::path::Path;

pub fn strip_timestamps(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            map.retain(|key, _| {
                !(key == "timestamp" || key.ends_with("_at") || key.ends_with("At"))
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
    for (_, value) in &mut records {
        normalize_lifecycle_metadata(value);
        normalize_values(value, &replacements);
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
            content_by_digest.insert(
                digest.to_owned(),
                normalized_published_content(&bytes, replacements),
            );
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
                if let Some(prior_content) = map
                    .get("prior_blake3")
                    .and_then(serde_json::Value::as_str)
                    .and_then(|digest| content_by_digest.get(digest))
                {
                    map.insert("prior_content".into(), prior_content.clone());
                    map.remove("prior_blake3");
                }
                map.remove("blake3");
                map.remove("byte_len");
                map.insert("content".into(), content);
            }
        }
    }
}

fn normalized_published_content(bytes: &[u8], replacements: &[(&str, &str)]) -> serde_json::Value {
    match serde_json::from_slice(bytes) {
        Ok(mut value) => {
            normalize_values(&mut value, replacements);
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
mod tests {
    use super::normalized_published_content;

    #[test]
    fn published_envelopes_ignore_root_paths_but_keep_findings() {
        let first = br#"{"project_root":"/tmp/one","findings":[{"rule":"task-set-lint","message":"missing task"}]}"#;
        let same_finding = br#"{"project_root":"/tmp/two","findings":[{"rule":"task-set-lint","message":"missing task"}]}"#;
        let different_finding = br#"{"project_root":"/tmp/two","findings":[{"rule":"task-set-lint","message":"different"}]}"#;
        let replacements_one = [("/tmp/one", "<project-root>")];
        let replacements_two = [("/tmp/two", "<project-root>")];

        assert_eq!(
            normalized_published_content(first, &replacements_one),
            normalized_published_content(same_finding, &replacements_two)
        );
        assert_ne!(
            normalized_published_content(first, &replacements_one),
            normalized_published_content(different_finding, &replacements_two)
        );
    }
}

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
