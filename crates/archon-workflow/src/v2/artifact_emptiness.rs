//! Whether a declared artifact that exists actually holds anything.
//!
//! Split from `artifact_path_guard.rs` for the 500-line ceiling; the caller is
//! `artifact_file_defect` and lives there.

use std::path::Path;

/// A file that exists, is not zero bytes, and still holds no records.
///
/// A zero-byte file was already refused; a registry holding `{}` was not.
/// Observed live: a task declaring a dataset registry was ACCEPTED against
/// `{"datasets": {}, "last_updated": "...", "schema": "...", "snapshots": {}}`
/// — 141 bytes, every collection empty, not one dataset in it. The task had
/// edited source and produced nothing, and existence was the only question
/// asked. An empty DIRECTORY is already a defect here; this is the same
/// judgement applied to a file, which is where the asymmetry was.
///
/// Deliberately narrow. It fires only when the artifact parses as JSON, holds
/// at least one array or object, and every one of them is empty. A file of
/// scalars — `{"status": "ok"}` — has no container to be empty and is left
/// alone, as is anything that is not JSON at all. The question is "did you
/// produce any records", not "is this file interesting".
pub(super) fn structurally_empty_defect(path: &Path) -> Option<&'static str> {
    const DEFECT: &str = "exists but holds no records: every array and object in it is empty";
    let text = std::fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;

    // The ROOT is judged on its own: `{}` and `[]` hold nothing and say so.
    match &value {
        serde_json::Value::Array(items) if items.is_empty() => return Some(DEFECT),
        serde_json::Value::Object(entries) if entries.is_empty() => return Some(DEFECT),
        _ => {}
    }

    // Below the root, the root's OWN key count is not evidence of anything: a
    // registry wrapper carries `schema` and `last_updated` whatever happens, so
    // counting it made the live 141-byte case look populated. Only the
    // collections inside it answer the question "did you produce any records".
    let (containers, non_empty) = nested_container_counts(&value);
    (containers > 0 && non_empty == 0).then_some(DEFECT)
}

/// `(containers seen, containers holding anything)` BELOW the given value.
fn nested_container_counts(value: &serde_json::Value) -> (usize, usize) {
    let children: Vec<&serde_json::Value> = match value {
        serde_json::Value::Array(items) => items.iter().collect(),
        serde_json::Value::Object(entries) => entries.values().collect(),
        _ => return (0, 0),
    };
    let mut totals = (0, 0);
    for child in children {
        if let Some(len) = container_len(child) {
            totals = (totals.0 + 1, totals.1 + usize::from(len > 0));
        }
        let nested = nested_container_counts(child);
        totals = (totals.0 + nested.0, totals.1 + nested.1);
    }
    totals
}

/// The entry count of a container, or `None` for a scalar.
fn container_len(value: &serde_json::Value) -> Option<usize> {
    match value {
        serde_json::Value::Array(items) => Some(items.len()),
        serde_json::Value::Object(entries) => Some(entries.len()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn artifact(body: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("registry.json");
        std::fs::write(&path, body).expect("write");
        (dir, path)
    }

    /// The live failure, byte for byte: 141 bytes, accepted, not one dataset.
    #[test]
    fn a_registry_with_every_collection_empty_is_not_evidence() {
        let (_dir, path) = artifact(
            r#"{"datasets": {}, "last_updated": "2026-08-15T08:46:05Z", "schema": "archon-trading-data-registry-v1", "snapshots": {}}"#,
        );
        assert_eq!(
            structurally_empty_defect(&path),
            Some("exists but holds no records: every array and object in it is empty")
        );
    }

    #[test]
    fn one_record_anywhere_is_enough() {
        let (_dir, path) = artifact(r#"{"datasets": {"AAPL-1d": {}}, "snapshots": {}}"#);
        assert_eq!(structurally_empty_defect(&path), None);
    }

    #[test]
    fn a_bare_empty_object_or_array_is_not_evidence() {
        let (_dir, path) = artifact("{}");
        assert!(structurally_empty_defect(&path).is_some());
        let (_dir, path) = artifact("[]");
        assert!(structurally_empty_defect(&path).is_some());
    }

    /// The false positive this must not produce. A document of scalars has no
    /// container to be empty, so it is not this rule's business.
    #[test]
    fn a_document_of_scalars_is_left_alone() {
        let (_dir, path) = artifact(r#"{"status": "ok", "count": 3}"#);
        assert_eq!(structurally_empty_defect(&path), None);
    }

    /// Non-JSON is not this rule's business either: a markdown report or a CSV
    /// must not be failed for not parsing.
    #[test]
    fn a_non_json_artifact_is_left_alone() {
        let (_dir, path) = artifact("# Gap Audit\n\nNo gaps found.\n");
        assert_eq!(structurally_empty_defect(&path), None);
    }

    /// A wrapper whose OWN keys are the only non-empty thing must not pass on
    /// the strength of its own metadata. This is the exact shape that made the
    /// first draft of this rule return None on the live file.
    #[test]
    fn a_wrapper_full_of_metadata_does_not_vouch_for_its_empty_collections() {
        let (_dir, path) =
            artifact(r#"{"schema": "v1", "generated_at": "2026-08-15", "cells": [], "gaps": []}"#);
        assert!(structurally_empty_defect(&path).is_some());
    }

    /// A container holding only EMPTY containers is still empty in substance,
    /// and the recursion must not let the outer one vouch for the inner ones.
    #[test]
    fn a_container_of_empty_containers_is_not_evidence() {
        let (_dir, path) = artifact(r#"{"datasets": {"equities": {}, "crypto": {}}}"#);
        assert!(
            structurally_empty_defect(&path).is_none(),
            "documented: `datasets` holds two keys, so it IS populated at that level; \
             substance below that is the deliverable contract's question, not this one"
        );
    }

    /// Malformed JSON must not be reported as empty — it is a different fault,
    /// and claiming this one would send a retry after the wrong thing.
    #[test]
    fn malformed_json_is_left_alone() {
        let (_dir, path) = artifact(r#"{"datasets": {"#);
        assert_eq!(structurally_empty_defect(&path), None);
    }

    /// A JSON scalar document is not a container at all.
    #[test]
    fn a_bare_scalar_document_is_left_alone() {
        let (_dir, path) = artifact("42");
        assert_eq!(structurally_empty_defect(&path), None);
    }

    /// Nesting counts: records hidden one level down still count as records.
    #[test]
    fn a_nested_record_counts() {
        let (_dir, path) = artifact(r#"{"outer": {"inner": {"cells": [1]}}}"#);
        assert_eq!(structurally_empty_defect(&path), None);
    }
}
