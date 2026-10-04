//! Real refusal envelopes exercise field-by-field convergence.
use super::tests::{assert_author_loop_keeps_running, envelope};
use super::*;
use serde_json::{Value, json};

fn subjects(value: &Value) -> std::collections::BTreeSet<String> {
    value["policy_findings"]
        .as_array()
        .expect("findings")
        .iter()
        .map(|finding| {
            assert_eq!(
                finding["deterministic_defect"]["code"],
                "invalid_candidate_shape"
            );
            let subject = &finding["deterministic_defect"]["subject"];
            subject.as_str().expect("subject").to_string()
        })
        .collect()
}

/// Runs `repairs` one per attempt on `candidate`: the defect count must fall
/// by one each time, the first envelope must name `first` exactly, and the
/// fixed script's author loop must keep running on the real envelopes.
fn assert_converges(
    shape: &ElementShape,
    mut candidate: Value,
    first: &[&str],
    repairs: &[(&str, Value)],
) {
    let dir = tempfile::tempdir().expect("fixture");
    let mut envelopes = Vec::new();
    for step in 0..=repairs.len() {
        let value = envelope(dir.path(), &format!("step-{step}"), &candidate, shape);
        let named = subjects(&value);
        assert_eq!(named.len(), repairs.len() - step, "step {step}: {value}");
        if step == 0 {
            let expected: std::collections::BTreeSet<_> =
                first.iter().map(|pointer| pointer.to_string()).collect();
            assert_eq!(named, expected, "{value}");
        }
        envelopes.push(value);
        if let Some((pointer, fill)) = repairs.get(step) {
            let (parent, key) = pointer.rsplit_once('/').expect("pointer");
            let target = candidate.pointer_mut(parent).expect("parent");
            target[key] = fill.clone();
        }
    }
    assert_author_loop_keeps_running(&envelopes);
}

#[test]
fn workflow_host_command_each_missing_consumed_artifact_path_is_its_own_identity() {
    let candidate = json!({ "schema_version": 1, "acceptance_digest": "d", "tasks": [{
        "task_id": "TASK-X-002", "file_name": "TASK-X-002.md",
        "depends_on": [{ "task_id": "TASK-X-001", "consumes": [{}, {}, {}] }] }] });
    let at = "/tasks/0/depends_on/0/consumes";
    let repairs: Vec<_> = (0..3)
        .map(|n| {
            (
                format!("{at}/{n}/artifact_path"),
                json!(format!("out{n}.json")),
            )
        })
        .collect();
    let repairs: Vec<_> = repairs
        .iter()
        .map(|(p, v)| (p.as_str(), v.clone()))
        .collect();
    let first: Vec<_> = repairs.iter().map(|(p, _)| &p[1..]).collect();
    assert_converges(&TASK_SHAPE, candidate, &first, &repairs);
}

/// Entries and supplementary entries alike: a command check's missing or
/// invalid field, each count. Missing tags are covered by the round 11 corpus.
#[test]
fn workflow_host_command_each_command_check_field_is_its_own_identity() {
    let candidate = json!({
        "entries": [{ "id": "AC-X-001", "criterion": "c", "check": { "kind": "command" } }],
        "supplementary": [{ "id": "SUP-X-001", "criterion": "c",
            "check": { "kind": "command", "command": "true", "cwd": "elsewhere" } }] });
    let repairs = [
        ("/entries/0/check/command", json!("true")),
        ("/entries/0/check/cwd", json!("project_root")),
        ("/supplementary/0/check/cwd", json!("repo_root")),
    ];
    let first: Vec<_> = repairs.iter().map(|(p, _)| &p[1..]).collect();
    assert_converges(&ENTRY_SHAPE, candidate, &first, &repairs);
}

/// A floor check's contract, missing or empty: each required contract field
/// is its own identity.
#[test]
fn workflow_host_command_each_floor_contract_field_is_its_own_identity() {
    let candidate = json!({
        "entries": [{ "id": "AC-X-001", "criterion": "c", "check": { "kind": "floor" } }],
        "supplementary": [{ "id": "SUP-X-001", "criterion": "c",
            "check": { "kind": "floor", "contract": {} } }] });
    let repairs = [
        ("/entries/0/check/contract", json!({})),
        ("/entries/0/check/contract/kind", json!("file")),
        ("/entries/0/check/contract/artifact_path", json!("a.json")),
        ("/supplementary/0/check/contract/kind", json!("file")),
        (
            "/supplementary/0/check/contract/artifact_path",
            json!("b.json"),
        ),
    ];
    let first = [
        "entries/0/check/contract",
        "entries/0/check/contract/kind",
        "entries/0/check/contract/artifact_path",
        "supplementary/0/check/contract/kind",
        "supplementary/0/check/contract/artifact_path",
    ];
    assert_converges(&ENTRY_SHAPE, candidate, &first, &repairs);
}
