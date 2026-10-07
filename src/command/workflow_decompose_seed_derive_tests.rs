//! Issue 360: what a phase seed carries, read from durable call records.
use super::*;
use serde_json::json;

pub(crate) fn record(
    id: &str,
    at: &str,
    call: Value,
    status: &str,
    data: Value,
) -> WorkflowV2CallRecord {
    serde_json::from_value(json!({
        "call": call,
        "attempt": 1,
        "started_at": format!("2026-10-06T{at}+00:00"),
        "input_hash": format!("input-{id}"),
        "status": status,
        "result": {
            "status": status, "summary": "", "evidence": [], "artifacts": [], "commands_run": [],
            "files_read": [], "files_changed": [], "task_coverage": [], "residual_gaps": [],
            "data": data,
        },
    }))
    .unwrap()
}

pub(crate) fn reply(id: &str, at: &str, content: &str) -> WorkflowV2CallRecord {
    record(
        id,
        at,
        json!({"id": id, "method": "agent", "options": {"result_mode": "rawOutcome"}}),
        "accepted",
        json!({"content": content, "stopReason": "end_turn"}),
    )
}

pub(crate) fn finding(text: &str, subject: &str) -> Value {
    json!({"text": text, "subject": subject, "remediation_scope": "candidate_artifact"})
}

pub(crate) fn gate(
    command: &str,
    at: &str,
    stdin: &str,
    findings: Vec<Value>,
    published: bool,
) -> WorkflowV2CallRecord {
    let id = format!("{command}-{at}");
    record(
        &id,
        at,
        json!({"id": id, "method": "hostCommand", "options": {"host_command": {"commandId": command, "stdin": stdin}}}),
        "needs_review",
        json!({
            "exitCode": 0, "stdout": "", "stderr": "", "stdoutBytes": 0, "stderrBytes": 0,
            "timedOut": false, "interrupted": false, "stdoutTruncated": false, "stderrTruncated": false,
            "gateEnvelope": {"schema_version": 1, "report": "judged", "policy_findings": findings},
            "publicationReceipt": if published { json!({"call_id": id}) } else { Value::Null },
            "postcondition": {"satisfied": published, "summary": "fixture"},
        }),
    )
}

/// A complete entry under this build's validator.
pub(crate) fn entry(id: &str) -> Value {
    json!({
        "id": id, "criterion": format!("criterion of {id}"),
        "check": {"kind": "command", "command": format!("check-{id}"), "cwd": "project_root"},
        "gap_permitted": false, "covers": [], "judgment": {"verdict": "accepted", "counterexample": "", "reason": "", "host_call_id": ""},
    })
}

/// Each criterion's text, as `entry` carries it.
fn criteria(ids: &[&str]) -> BTreeMap<String, String> {
    ids.iter()
        .map(|id| (id.to_string(), format!("criterion of {id}")))
        .collect()
}

fn entries_seed(
    derived: &Derived,
) -> (
    &Vec<GateSeed>,
    &Option<String>,
    &Vec<ReplySeed>,
    &BTreeMap<String, Vec<String>>,
    usize,
) {
    match &derived.subjects["acceptance"] {
        SubjectSeed::Entries {
            gates,
            candidate,
            replies,
            invalid,
            carried,
        } => (gates, candidate, replies, invalid, *carried),
        other => panic!("{other:?}"),
    }
}

#[test]
fn the_last_gate_candidate_is_carried_with_the_replies_authored_since() {
    let mut repaired = entry("AC-2");
    repaired["check"]["command"] = json!("stronger");
    let candidate =
        json!({"entries": [entry("AC-1"), entry("AC-2")], "supplementary": []}).to_string();
    let records = vec![
        reply(
            "acceptance-author-AC-1-4",
            "01:00:00",
            &entry("AC-1").to_string(),
        ),
        reply(
            "acceptance-author-AC-2-4",
            "01:01:00",
            &entry("AC-2").to_string(),
        ),
        gate(
            "freeze-acceptance",
            "02:00:00",
            &candidate,
            vec![finding(
                "check 'AC-2' was refuted by the host judge; reason: weak",
                "AC-2",
            )],
            false,
        ),
        // Round 2 re-authored AC-2 after the gate, fenced as authors do.
        reply(
            "acceptance-author-AC-2-7",
            "03:00:00",
            &format!("```json\n{repaired}\n```"),
        ),
    ];
    let derived = derive(&records, &[], &criteria(&["AC-1", "AC-2"])).unwrap();
    let (gates, carried_candidate, replies, invalid, carried) = entries_seed(&derived);
    assert_eq!(gates.len(), 1);
    assert_eq!(
        carried_candidate.as_deref(),
        Some(candidate.as_str()),
        "the gate's exact bytes"
    );
    assert_eq!(replies.len(), 1);
    assert_eq!(replies[0].call_id, "acceptance-author-AC-2-7");
    assert_eq!(
        serde_json::from_str::<Value>(&replies[0].text).unwrap(),
        repaired
    );
    assert!(invalid.is_empty(), "{invalid:?}");
    assert_eq!(carried, 2);
    assert_eq!(derived.author_ordinals["acceptance"], 7);
}

/// Issue 357 stamps the host criterion before it keeps an entry, so the
/// gate's candidate holds the criterion text while the replies hold the
/// author's. The gate still holds that round: no reply is "since" it.
#[test]
fn a_gate_holds_the_round_whose_replies_it_stamped() {
    let raw = |id: &str| {
        let mut entry = entry(id);
        entry["criterion"] = json!("the author's own words");
        entry.to_string()
    };
    let candidate = json!({"entries": [entry("AC-1"), entry("AC-2")]}).to_string();
    let records = vec![
        reply("acceptance-author-AC-1-4", "01:00:00", &raw("AC-1")),
        reply("acceptance-author-AC-2-4", "01:01:00", &raw("AC-2")),
        gate(
            "freeze-acceptance",
            "02:00:00",
            &candidate,
            Vec::new(),
            false,
        ),
    ];
    let derived = derive(&records, &[], &criteria(&["AC-1", "AC-2"])).unwrap();
    let (_, _, replies, invalid, carried) = entries_seed(&derived);
    assert!(replies.is_empty(), "{replies:?}");
    assert!(invalid.is_empty(), "{invalid:?}");
    assert_eq!(carried, 2);
}

#[test]
fn a_carried_entry_this_build_refuses_is_named_invalid() {
    let mut bad = entry("SUP-REQ-1");
    bad.as_object_mut().unwrap().remove("check");
    // The criterion is host-owned (Issue 357): the author step sets it before
    // it validates, so an entry carried without one is stamped, not refused.
    let mut unstamped = entry("AC-1");
    unstamped.as_object_mut().unwrap().remove("criterion");
    let mut no_criterion = bad.clone();
    no_criterion.as_object_mut().unwrap().remove("criterion");
    let candidate = json!({"entries": [unstamped], "supplementary": [no_criterion]}).to_string();
    let owed = finding(
        "check 'SUP-REQ-1': PRD requirement REQ-1 is covered by no acceptance check; author supplementary check SUP-REQ-1 with covers [\"REQ-1\"] that fails whenever REQ-1 is violated: the requirement",
        "SUP-REQ-1",
    );
    let records = vec![
        gate(
            "freeze-acceptance",
            "01:00:00",
            &json!({"entries": [entry("AC-1")]}).to_string(),
            vec![owed],
            false,
        ),
        gate(
            "freeze-acceptance",
            "02:00:00",
            &candidate,
            vec![finding(
                "candidate artifact was refused: supplementary/0/check is missing or has an invalid type or value",
                "acceptance",
            )],
            false,
        ),
    ];
    let derived = derive(&records, &[], &criteria(&["AC-1"])).unwrap();
    let (gates, _, _, invalid, carried) = entries_seed(&derived);
    assert_eq!(gates.len(), 2, "every gate, for the owed checks they name");
    assert_eq!(carried, 2);
    assert_eq!(invalid.keys().collect::<Vec<_>>(), ["SUP-REQ-1"]);
    assert!(
        invalid["SUP-REQ-1"]
            .iter()
            .all(|refusal| refusal.contains("entry/check") && !refusal.contains("criterion")),
        "{invalid:?}"
    );
}

#[test]
fn a_reply_repairing_the_refused_entry_is_checked_after_the_host_owned_fields() {
    let mut bad = entry("SUP-REQ-1");
    bad.as_object_mut().unwrap().remove("criterion");
    let owed = finding(
        "check 'SUP-REQ-1': PRD requirement REQ-1 is covered by no acceptance check; author it: text",
        "SUP-REQ-1",
    );
    let mut repaired = entry("SUP-REQ-1");
    repaired["covers"] = json!(["REQ-OTHER"]);
    repaired["gap_permitted"] = json!(true);
    let records = vec![
        gate(
            "freeze-acceptance",
            "01:00:00",
            &json!({"entries": [], "supplementary": [bad]}).to_string(),
            vec![owed],
            false,
        ),
        reply(
            "acceptance-author-SUP-REQ-1-7",
            "02:00:00",
            &format!("Here it is: {repaired}"),
        ),
    ];
    let derived = derive(&records, &[], &criteria(&[])).unwrap();
    let (_, _, replies, invalid, _) = entries_seed(&derived);
    assert!(
        invalid.is_empty(),
        "the repair replaces the refused entry: {invalid:?}"
    );
    assert_eq!(
        replies[0].text,
        repaired.to_string(),
        "the object the script extracts"
    );
}

#[test]
fn a_phase_never_frozen_carries_its_replies_alone() {
    let records = vec![
        reply(
            "acceptance-author-AC-1-4",
            "01:00:00",
            &entry("AC-1").to_string(),
        ),
        reply("acceptance-author-AC-2-4", "01:00:01", "no entry here"),
        record(
            "acceptance-author-AC-3-4",
            "01:00:02",
            json!({"id": "acceptance-author-AC-3-4", "method": "agent"}),
            "running",
            Value::Null,
        ),
    ];
    let derived = derive(&records, &[], &criteria(&["AC-1", "AC-2", "AC-3"])).unwrap();
    let (gates, candidate, replies, invalid, carried) = entries_seed(&derived);
    assert!(gates.is_empty() && candidate.is_none());
    assert_eq!(
        replies.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        ["AC-1"]
    );
    assert!(invalid.is_empty());
    assert_eq!(carried, 1, "AC-2 and AC-3 are authored again");
    assert_eq!(
        derived.author_ordinals["acceptance"], 4,
        "an unfinished call holds its ordinal"
    );
}

#[test]
fn a_supplementary_check_no_gate_owed_pauses_the_resume() {
    let candidate = json!({"entries": [], "supplementary": [entry("SUP-REQ-9")]}).to_string();
    let records = vec![gate(
        "freeze-acceptance",
        "01:00:00",
        &candidate,
        Vec::new(),
        false,
    )];
    let error = derive(&records, &[], &criteria(&[])).unwrap_err();
    assert!(format!("{error:#}").contains("paused"), "{error:#}");
}

#[test]
fn an_artifact_carries_its_latest_reply_and_the_gate_that_judged_those_bytes() {
    let records = vec![
        reply("skeleton-author-1", "01:00:00", "skeleton one"),
        gate(
            "freeze-skeleton",
            "01:10:00",
            "skeleton one",
            vec![finding("task lacks an owner", "skeleton")],
            false,
        ),
        reply("skeleton-author-2", "02:00:00", "skeleton two"),
        gate(
            "freeze-skeleton",
            "02:10:00",
            "skeleton two",
            Vec::new(),
            true,
        ),
        reply("body-TASK-1-author-1", "03:00:00", "body one"),
        gate(
            "land-task-body",
            "03:10:00",
            "body one",
            vec![finding("observation missing", "TASK-1")],
            false,
        ),
        reply("body-TASK-2-author-3", "04:00:00", "body two, never judged"),
        record(
            "body-TASK-2-author-4",
            "05:00:00",
            json!({"id": "body-TASK-2-author-4", "method": "agent"}),
            "failed",
            Value::Null,
        ),
    ];
    let derived = derive(
        &records,
        &["pause-body-TASK-2-2".into(), "pause-set-gates-1".into()],
        &criteria(&[]),
    )
    .unwrap();
    let artifact = |subject: &str| match &derived.subjects[subject] {
        SubjectSeed::Artifact {
            candidate, gate, ..
        } => (candidate.clone(), gate.clone()),
        other => panic!("{other:?}"),
    };
    let (skeleton, judged) = artifact("skeleton");
    assert_eq!(skeleton, "skeleton two");
    assert!(judged.unwrap().published);
    let (body, judged) = artifact("body-TASK-1");
    assert_eq!(body, "body one");
    assert_eq!(judged.unwrap().findings.len(), 1);
    let (body, judged) = artifact("body-TASK-2");
    assert_eq!(body, "body two, never judged");
    assert!(judged.is_none());
    assert_eq!(
        derived.author_ordinals["body-TASK-2"], 4,
        "a failed call holds its ordinal"
    );
    assert_eq!(derived.pause_ordinals["body-TASK-2"], 2);
    assert_eq!(derived.pause_ordinals["set-gates"], 1);
    assert!(!derived.subjects.contains_key("acceptance"));
}

#[test]
fn an_operational_gate_judged_nothing() {
    let mut outage = gate(
        "freeze-skeleton",
        "02:00:00",
        "skeleton one",
        Vec::new(),
        false,
    );
    outage.result.data["gateEnvelope"]["operational_error"] =
        json!({"kind": "operational", "text": "judge truncated"});
    let records = vec![
        reply("skeleton-author-1", "01:00:00", "skeleton one"),
        outage,
    ];
    let derived = derive(&records, &[], &criteria(&[])).unwrap();
    assert!(matches!(
        &derived.subjects["skeleton"],
        SubjectSeed::Artifact { gate: None, .. }
    ));
}

#[test]
fn reply_extraction_reads_what_the_script_reads() {
    assert_eq!(
        extract_object("```json\n{\"id\":\"A\"}\n```"),
        "{\"id\":\"A\"}"
    );
    assert_eq!(
        extract_object("prose {\"id\":\"A\"} more"),
        "{\"id\":\"A\"}"
    );
    let wrapped = json!({"schema_version": 1, "acceptance": [{"id": "A"}]}).to_string();
    assert_eq!(reply_entry(&wrapped, "A"), Some(json!({"id": "A"})));
    assert_eq!(reply_entry("{\"id\":\"B\"}", "A"), None);
    assert_eq!(
        author_call("acceptance-author-SUP-REQ-DL-133-19"),
        Some(("acceptance".into(), Some("SUP-REQ-DL-133".into()), 19))
    );
    assert_eq!(
        author_call("body-TASK-X-010-author-5"),
        Some(("body-TASK-X-010".into(), None, 5))
    );
    assert_eq!(author_call("acceptance-author-7"), None);
    assert_eq!(
        pause_ordinal("pause-body-TASK-1-3"),
        Some(("body-TASK-1".into(), 3))
    );
}

#[path = "workflow_decompose_seed_order_tests.rs"]
mod order;
