use std::collections::{BTreeMap, BTreeSet};

use archon_core::agents::AgentMetadata;
use blake3::Hash;
use serde_json::Value;

/// Computes a deterministic checksum for a sorted index and its member sets.
///
/// Callers canonicalize unordered map/set buckets before invoking this helper,
/// which makes equivalent indexes produce the same checksum regardless of
/// their original iteration order.
#[must_use]
pub fn deterministic_index_checksum(index: &BTreeMap<String, BTreeSet<String>>) -> u64 {
    index
        .iter()
        .fold(0xcbf2_9ce4_8422_2325, |checksum, (key, members)| {
            let checksum = checksum_part(checksum, key);
            members
                .iter()
                .fold(checksum, |checksum, member| checksum_part(checksum, member))
        })
}

/// Serializes every metadata field into recursively key-sorted JSON and hashes it.
///
/// This makes semantically equivalent JSON schemas produce the same digest even
/// when their object keys were inserted in a different order.
#[must_use]
pub fn metadata_digest(metadata: &AgentMetadata) -> Hash {
    let value = serde_json::to_value(metadata).expect("AgentMetadata serializes");
    blake3::hash(canonical_json(&value).as_bytes())
}

/// Recursively canonicalizes JSON object keys while preserving array order.
#[must_use]
pub fn canonical_json(value: &Value) -> String {
    match value {
        Value::Null => "null".to_owned(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => serde_json::to_string(value).expect("string serializes"),
        Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(canonical_json)
                .collect::<Vec<_>>()
                .join(",")
        ),
        Value::Object(values) => {
            let mut fields = values.iter().collect::<Vec<_>>();
            fields.sort_unstable_by_key(|(key, _)| *key);
            let fields = fields
                .into_iter()
                .map(|(key, value)| {
                    format!(
                        "{}:{}",
                        serde_json::to_string(key).expect("object key serializes"),
                        canonical_json(value)
                    )
                })
                .collect::<Vec<_>>();
            format!("{{{}}}", fields.join(","))
        }
    }
}

fn checksum_part(checksum: u64, value: &str) -> u64 {
    value
        .bytes()
        .chain([0xff])
        .fold(checksum, |checksum, byte| {
            checksum.wrapping_mul(0x0000_0100_0000_01b3) ^ u64::from(byte)
        })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use archon_core::agents::{AgentState, DependencyRef, ResourceReq, SourceKind};
    use chrono::{DateTime, Utc};
    use serde_json::json;

    use super::*;

    fn metadata() -> AgentMetadata {
        AgentMetadata {
            name: "agent".to_owned(),
            version: semver::Version::new(1, 2, 3),
            description: "description".to_owned(),
            category: "category".to_owned(),
            tags: vec!["tag".to_owned()],
            capabilities: vec!["capability".to_owned()],
            input_schema: json!({"input": {"type": "string"}}),
            output_schema: json!({"output": {"type": "string"}}),
            resource_requirements: ResourceReq {
                cpu: 2.0,
                memory_mb: 512,
                timeout_sec: 60,
            },
            dependencies: vec![DependencyRef {
                name: "dependency".to_owned(),
                version_req: "^1.0".parse().expect("valid version requirement"),
            }],
            source_path: PathBuf::from("/fixture/agent.toml"),
            source_kind: SourceKind::Local,
            state: AgentState::Valid,
            loaded_at: DateTime::<Utc>::from_timestamp(1_700_000_000, 0)
                .expect("valid fixed timestamp"),
        }
    }

    #[test]
    fn metadata_digest_changes_for_each_metadata_field() {
        let original = metadata();
        let cases: Vec<(&str, Box<dyn Fn(&mut AgentMetadata)>)> = vec![
            ("name", Box::new(|value| value.name.push('x'))),
            ("version", Box::new(|value| value.version.patch += 1)),
            ("description", Box::new(|value| value.description.push('x'))),
            ("category", Box::new(|value| value.category.push('x'))),
            (
                "tags",
                Box::new(|value| value.tags.push("new-tag".to_owned())),
            ),
            (
                "capabilities",
                Box::new(|value| value.capabilities.push("new-capability".to_owned())),
            ),
            (
                "input_schema",
                Box::new(|value| value.input_schema = json!({"changed": true})),
            ),
            (
                "output_schema",
                Box::new(|value| value.output_schema = json!({"changed": true})),
            ),
            (
                "resource_requirements",
                Box::new(|value| value.resource_requirements.memory_mb += 1),
            ),
            (
                "dependencies",
                Box::new(|value| value.dependencies[0].name.push('x')),
            ),
            (
                "source_path",
                Box::new(|value| value.source_path.push("changed")),
            ),
            (
                "source_kind",
                Box::new(|value| value.source_kind = SourceKind::Remote),
            ),
            (
                "state",
                Box::new(|value| value.state = AgentState::Invalid("changed".to_owned())),
            ),
            (
                "loaded_at",
                Box::new(|value| {
                    value.loaded_at = DateTime::<Utc>::from_timestamp(1_700_000_001, 0)
                        .expect("valid fixed timestamp");
                }),
            ),
        ];

        for (field, mutate) in cases {
            let mut changed = original.clone();
            mutate(&mut changed);
            assert_ne!(
                metadata_digest(&original),
                metadata_digest(&changed),
                "{field}"
            );
        }
    }

    #[test]
    fn canonical_json_sorts_nested_object_keys() {
        let left = json!({"z": {"b": 2, "a": 1}, "a": [2, 1]});
        let right = json!({"a": [2, 1], "z": {"a": 1, "b": 2}});

        assert_eq!(canonical_json(&left), canonical_json(&right));
    }

    #[test]
    fn index_checksum_detects_changed_members() {
        let left = BTreeMap::from([(
            "tag-0".to_owned(),
            BTreeSet::from(["agent-0@1.0.0".to_owned()]),
        )]);
        let right = BTreeMap::from([(
            "tag-0".to_owned(),
            BTreeSet::from(["agent-999@9.9.9".to_owned()]),
        )]);

        assert_ne!(
            deterministic_index_checksum(&left),
            deterministic_index_checksum(&right)
        );
    }

    #[test]
    fn index_checksum_is_independent_of_input_order_after_canonicalization() {
        let first = BTreeMap::from([
            (
                "tag-1".to_owned(),
                BTreeSet::from(["agent-1@1.0.0".to_owned(), "agent-0@1.0.0".to_owned()]),
            ),
            (
                "tag-0".to_owned(),
                BTreeSet::from(["agent-2@1.0.0".to_owned()]),
            ),
        ]);
        let second = BTreeMap::from([
            (
                "tag-0".to_owned(),
                BTreeSet::from(["agent-2@1.0.0".to_owned()]),
            ),
            (
                "tag-1".to_owned(),
                BTreeSet::from(["agent-0@1.0.0".to_owned(), "agent-1@1.0.0".to_owned()]),
            ),
        ]);

        assert_eq!(
            deterministic_index_checksum(&first),
            deterministic_index_checksum(&second)
        );
    }
}
