use std::collections::BTreeMap;

use sha2::{Digest, Sha256};

pub(super) fn stable_hash(value: &serde_json::Value) -> String {
    let canonical = canonical_json(value);
    let bytes = serde_json::to_vec(&canonical).unwrap_or_default();
    let digest = Sha256::digest(bytes);
    format!("{digest:x}")
}

fn canonical_json(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Array(items) => {
            let mut values = items.iter().map(canonical_json).collect::<Vec<_>>();
            values.sort_by(|left, right| {
                serde_json::to_string(left)
                    .unwrap_or_default()
                    .cmp(&serde_json::to_string(right).unwrap_or_default())
            });
            serde_json::Value::Array(values)
        }
        serde_json::Value::Object(object) => {
            let mut sorted = serde_json::Map::new();
            for (key, value) in object.iter().collect::<BTreeMap<_, _>>() {
                sorted.insert(key.clone(), canonical_json(value));
            }
            serde_json::Value::Object(sorted)
        }
        other => other.clone(),
    }
}
