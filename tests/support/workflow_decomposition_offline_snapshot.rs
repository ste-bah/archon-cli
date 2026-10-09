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
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            let mut value: serde_json::Value =
                serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            strip_timestamps(&mut value);
            normalize_values(
                &mut value,
                &[
                    (root.to_str().unwrap(), "<project-root>"),
                    (endpoint, "<provider-endpoint>"),
                    (run_id, "<run-id>"),
                ],
            );
            Some((name, value))
        })
        .flatten()
        .collect::<Vec<_>>();
    records.sort_by(|left, right| left.0.cmp(&right.0));
    records
}
