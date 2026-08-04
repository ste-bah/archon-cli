//! What [`super::stable_value_hash`] promises about workflow JSON payloads.
//!
//! The hash used to canonicalize the value first: sort object keys, and sort
//! ARRAY ELEMENTS recursively at every depth. Both were wrong-to-useless.
//!
//! Object-key sorting was redundant. `serde_json`'s `preserve_order` feature is
//! not enabled in this workspace, so `Map` is a `BTreeMap` and key order is
//! already deterministic on serialization.
//!
//! Array sorting was an outright defect. Array ORDER is semantically meaningful
//! in exactly the payloads hashed here, so `[a, b]` and `[b, a]` hashing alike
//! conflated genuinely different inputs and let a reordered run replay the wrong
//! cached result:
//!
//! - Array position IS fan-out branch identity: `fanout_item_id` falls back to
//!   the item index when an item declares no id/task_id/work_unit_id, and that
//!   index becomes the branch call id. Permuting `source_data` reassigns every
//!   branch while a sorting hash stays identical.
//! - Reducer arguments are a positional tuple consumed positionally in Rust
//!   (index 1 items, 2 result-with-outcomes, 3 verification records).
//! - Declared artifact verifiers execute in array order, fail-fast.
//!
//! `source_data` is only stripped from the hashed input when a source
//! fingerprint exists, and fingerprints are minted for a small set of wave ids
//! only — so every `reduce`, every `finalReport` and the whole v3 path hashed
//! raw `source_data` under the sorting rule.

use super::stable_value_hash;

#[test]
fn permuted_arrays_hash_differently() {
    // Array position is branch identity, a positional reducer argument, and
    // verifier execution order. A permutation is a DIFFERENT input.
    let left = serde_json::json!({ "source_data": [{ "item_id": "a" }, { "item_id": "b" }] });
    let right = serde_json::json!({ "source_data": [{ "item_id": "b" }, { "item_id": "a" }] });
    assert_ne!(stable_value_hash(&left), stable_value_hash(&right));
}

#[test]
fn permuted_nested_arrays_hash_differently() {
    let left = serde_json::json!({ "outer": [{ "inner": ["a", "b"] }] });
    let right = serde_json::json!({ "outer": [{ "inner": ["b", "a"] }] });
    assert_ne!(stable_value_hash(&left), stable_value_hash(&right));
}

#[test]
fn object_key_order_is_already_canonical() {
    // `preserve_order` is off, so both literals build the same BTreeMap.
    let left = serde_json::json!({ "alpha": 1, "beta": 2 });
    let right = serde_json::json!({ "beta": 2, "alpha": 1 });
    assert_eq!(stable_value_hash(&left), stable_value_hash(&right));
}

#[test]
fn identical_values_hash_identically() {
    let value = serde_json::json!({ "call_id": "implement-task-001", "source_data": [1, 2, 3] });
    assert_eq!(stable_value_hash(&value), stable_value_hash(&value.clone()));
}
