//! Issue 261 round 10: the required-field table is complete, and serde agrees
//! with it field by field.
use super::shape::table_defects;
use super::tests::{assert_author_loop_keeps_running, envelope};
use super::*;
use archon_workflow::task_set_contract::{
    AcceptanceCheck, AcceptanceCriterion, JudgeDecision, JudgeVerdict, TrustedCwd,
};
use archon_workflow::task_skeleton::{ConsumedArtifact, FrozenDependency, FrozenTask};
use archon_workflow::task_universe::WorkflowV2DeliverableContract;
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
/// invalid field, and a missing tag the present fields name, each count.
#[test]
fn workflow_host_command_each_command_check_field_is_its_own_identity() {
    let candidate = json!({
        "entries": [{ "id": "AC-X-001", "criterion": "c", "check": { "kind": "command" } }],
        "supplementary": [{ "id": "SUP-X-001", "criterion": "c",
            "check": { "command": "true", "cwd": "elsewhere" } }] });
    let repairs = [
        ("/entries/0/check/command", json!("true")),
        ("/entries/0/check/cwd", json!("project_root")),
        ("/supplementary/0/check/kind", json!("command")),
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
        ("/entries/0/check/contract", json!({ "kind": "file" })),
        ("/entries/0/check/contract/artifact_path", json!("a.json")),
        ("/supplementary/0/check/contract/kind", json!("file")),
        (
            "/supplementary/0/check/contract/artifact_path",
            json!("b.json"),
        ),
    ];
    let first = [
        "entries/0/check/contract/kind",
        "entries/0/check/contract/artifact_path",
        "supplementary/0/check/contract/kind",
        "supplementary/0/check/contract/artifact_path",
    ];
    assert_converges(&ENTRY_SHAPE, candidate, &first, &repairs);
}

/// One complete instance of every shape the precheck reads, each variant
/// included: the exhaustive match fails to compile when a variant is added.
fn samples() -> Vec<(&'static ElementShape, &'static str, Value)> {
    let contract = WorkflowV2DeliverableContract {
        kind: "file".into(),
        artifact_path: "out.json".into(),
        typed_verifier_command: Some("true".into()),
        ..Default::default()
    };
    let task = FrozenTask {
        task_id: "TASK-X-002".into(),
        file_name: "TASK-X-002.md".into(),
        depends_on: vec![FrozenDependency {
            task_id: "TASK-X-001".into(),
            consumes: vec![ConsumedArtifact {
                artifact_path: "in.json".into(),
                instance_source_records_field: Some("records".into()),
                registry_records_field: Some("records".into()),
                kind: Some("file".into()),
            }],
            ordering_only: true,
        }],
        blocks: vec!["TASK-X-003".into()],
        implements: vec!["REQ-1".into()],
        deliverable_contracts: vec![contract.clone()],
    };
    let checks = [
        AcceptanceCheck::Command {
            command: "true".into(),
            cwd: TrustedCwd::ProjectRoot,
        },
        AcceptanceCheck::Floor {
            contract: Box::new(contract),
        },
    ];
    let mut out = vec![(
        &TASK_SHAPE,
        "tasks",
        serde_json::to_value(task).expect("task"),
    )];
    for check in checks {
        match &check {
            AcceptanceCheck::Command { .. } | AcceptanceCheck::Floor { .. } => {}
        }
        let entry = AcceptanceCriterion {
            id: "AC-X-001".into(),
            criterion: "criterion".into(),
            check,
            gap_permitted: true,
            judgment: JudgeVerdict {
                verdict: JudgeDecision::Accepted,
                counterexample: String::new(),
                reason: String::new(),
                host_call_id: String::new(),
                sampling: None,
            },
            covers: vec!["REQ-1".into()],
        };
        let entry = serde_json::to_value(entry).expect("entry");
        out.push((&ENTRY_SHAPE, "entries", entry.clone()));
        out.push((&ENTRY_SHAPE, "supplementary", entry));
    }
    out
}

/// serde's read, exactly as the precheck runs it.
fn serde_reads(shape: &ElementShape, item: &Value) -> bool {
    (shape.serde_read)(item).is_ok()
}

/// The table's report alone: the precheck runs serde only when it is empty.
fn table_reports(shape: &ElementShape, list: &str, item: &Value) -> Vec<String> {
    table_defects(item, &format!("{list}/0"), shape.element)
        .into_iter()
        .map(|defect| defect.identity.subject)
        .collect()
}

fn pointers(value: &Value, at: String, out: &mut Vec<String>) {
    let children: Vec<(String, &Value)> = match value {
        Value::Object(map) => map.iter().map(|(k, v)| (k.clone(), v)).collect(),
        Value::Array(items) => items
            .iter()
            .enumerate()
            .map(|(n, v)| (n.to_string(), v))
            .collect(),
        _ => Vec::new(),
    };
    for (key, child) in children {
        let pointer = format!("{at}/{key}");
        out.push(pointer.clone());
        pointers(child, pointer, out);
    }
}

/// Drift guard: for every field of every sample, removing it, or writing a
/// value no variant names into it, is refused by serde exactly when the table
/// reports it, and the table names the field it reports. A required serde
/// field the table omits, or a table entry serde does not require, fails here.
#[test]
fn workflow_host_command_required_field_table_matches_serde() {
    let mut drift = Vec::new();
    for (shape, list, sample) in samples() {
        assert!(serde_reads(shape, &sample), "{sample}");
        assert_eq!(table_reports(shape, list, &sample), Vec::<String>::new());
        let mut all = Vec::new();
        pointers(&sample, String::new(), &mut all);
        for pointer in all {
            let (parent, key) = pointer.rsplit_once('/').expect("pointer");
            let mut probes = Vec::new();
            if let Some(Value::Object(_)) = sample.pointer(parent) {
                let mut removed = sample.clone();
                removed
                    .pointer_mut(parent)
                    .and_then(Value::as_object_mut)
                    .expect("parent")
                    .remove(key);
                probes.push(("removed", removed));
            }
            if sample.pointer(&pointer).is_some_and(Value::is_string) {
                let mut bogus = sample.clone();
                *bogus.pointer_mut(&pointer).expect("leaf") = json!("not-a-declared-value");
                probes.push(("unnamed value", bogus));
            }
            for (probe, item) in probes {
                let refused = !serde_reads(shape, &item);
                let reported = table_reports(shape, list, &item);
                let named = format!("{list}/0{pointer}");
                if refused != !reported.is_empty()
                    || reported.iter().any(|subject| !subject.starts_with(&named))
                {
                    drift.push(format!(
                        "{named} {probe}: serde refuses={refused}, table reports {reported:?}"
                    ));
                }
            }
            let mut retyped = sample.clone();
            *retyped.pointer_mut(&pointer).expect("field") = json!(0);
            if !table_reports(shape, list, &retyped).is_empty() && serde_reads(shape, &retyped) {
                drift.push(format!(
                    "{list}/0{pointer}: table refuses a type serde reads"
                ));
            }
        }
    }
    assert!(
        drift.is_empty(),
        "table and serde disagree:\n{}",
        drift.join("\n")
    );
}
