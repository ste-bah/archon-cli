//! Batch O: every review finding carries a host-stamped `finding_id`.
//!
//! Remediation used to count a whole group of findings resolved on one
//! verdict, and the terminal rule cleared every finding that named a task as
//! soon as that task had any remediation outcome. Resolution is now judged
//! per finding, so each finding needs an identity the host computes and no
//! agent can choose: a digest of the finding's whole canonical text (every
//! field, keys sorted, the `finding_id` field itself excluded). A finding
//! an agent stamped with an id of its own is re-stamped: the host's digest
//! is the only id any rule reads.

use serde_json::{Map, Value};

/// The field the host stamps on every finding it attaches or plans.
pub const FINDING_ID_KEY: &str = "finding_id";

/// The host's id of `finding`: `F-` and the first 16 hex digits of the
/// blake3 digest of its canonical text without [`FINDING_ID_KEY`]. Whatever
/// the finding already carries under that key is ignored.
pub fn finding_id_of(finding: &Value) -> String {
    let mut bare = finding.clone();
    if let Some(object) = bare.as_object_mut() {
        object.remove(FINDING_ID_KEY);
    }
    let digest = blake3::hash(canonical_text(&bare).as_bytes()).to_hex();
    format!("F-{}", &digest[..16])
}

/// `finding` with its host id stamped (an object; anything else is
/// returned unchanged, since the host wraps bare text before stamping).
pub fn stamp_finding_id(finding: Value) -> Value {
    let id = finding_id_of(&finding);
    match finding {
        Value::Object(mut object) => {
            object.insert(FINDING_ID_KEY.to_string(), Value::String(id));
            Value::Object(object)
        }
        other => other,
    }
}

/// JSON text with every object's keys sorted, whatever map order the
/// serializer was built with, so the digest never depends on field order.
pub fn canonical_text(value: &Value) -> String {
    canonical(value).to_string()
}

fn canonical(value: &Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut keys: Vec<&String> = object.keys().collect();
            keys.sort();
            let mut sorted = Map::new();
            for key in keys {
                sorted.insert(key.clone(), canonical(&object[key]));
            }
            Value::Object(sorted)
        }
        Value::Array(items) => Value::Array(items.iter().map(canonical).collect()),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_id_is_the_digest_of_the_whole_finding_whatever_its_key_order() {
        let a = json!({"claim": "x", "canonical_task_ids": ["T1"], "evidence": {"b": 1, "a": 2}});
        let b = json!({"evidence": {"a": 2, "b": 1}, "canonical_task_ids": ["T1"], "claim": "x"});
        assert_eq!(finding_id_of(&a), finding_id_of(&b));
        assert!(finding_id_of(&a).starts_with("F-"));
        // Any change to any field is a different finding.
        let c = json!({"claim": "x ", "canonical_task_ids": ["T1"], "evidence": {"b": 1, "a": 2}});
        assert_ne!(finding_id_of(&a), finding_id_of(&c));
    }

    #[test]
    fn an_agent_supplied_id_is_ignored_and_restamped() {
        let plain = json!({"claim": "x"});
        let forged = json!({"claim": "x", "finding_id": "F-0000000000000000"});
        assert_eq!(finding_id_of(&forged), finding_id_of(&plain));
        let stamped = stamp_finding_id(forged);
        assert_eq!(stamped[FINDING_ID_KEY], json!(finding_id_of(&plain)));
        // Stamping is idempotent.
        assert_eq!(stamp_finding_id(stamped.clone()), stamped);
    }
}
