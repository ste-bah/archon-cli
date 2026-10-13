use super::*;
use crate::v2::host_command::{HostCommandRequest, HostCommandSubject};
use crate::{WorkflowV2HostCall, WorkflowV2HostOptions, WorkflowV2Result, WorkflowV2Status};

fn request(task: &str, body: &str) -> HostCommandRequest {
    HostCommandRequest {
        command_id: "land".into(),
        stdin: Some(body.into()),
        target: Some(HostCommandSubject {
            task_id: task.into(),
            file_name: format!("{task}.md"),
        }),
    }
}

/// A landing record: `published` gives it a receipt; `refused` a body
/// finding.
fn landing(
    id: &str,
    at: &str,
    task: &str,
    body: &str,
    published: bool,
    refused: bool,
) -> WorkflowV2CallRecord {
    let findings: Vec<serde_json::Value> = if refused {
        vec![serde_json::json!({"remediation_scope": "body", "text": "gap"})]
    } else {
        vec![serde_json::json!({"remediation_scope": "inherited_predecessor", "text": "note"})]
    };
    let receipt = if published {
        serde_json::json!({"call_id": id, "entries": []})
    } else {
        serde_json::Value::Null
    };
    WorkflowV2CallRecord {
        run_id: "wf-lineage".into(),
        call: WorkflowV2HostCall {
            id: id.into(),
            method: WorkflowV2HostMethod::HostCommand,
            write_mode: None,
            options: WorkflowV2HostOptions {
                host_command: Some(request(task, body)),
                ..WorkflowV2HostOptions::default()
            },
        },
        attempt: 1,
        schema_version: "1".into(),
        admission_sequence: None,
        started_at: at.into(),
        finished_at: at.into(),
        input_hash: "in".into(),
        output_hash: "out".into(),
        status: if refused {
            WorkflowV2Status::NeedsReview
        } else {
            WorkflowV2Status::Accepted
        },
        result: WorkflowV2Result {
            data: serde_json::json!({
                "gateEnvelope": {"policy_findings": findings, "operational_error": null},
                "publicationReceipt": receipt,
            }),
            ..WorkflowV2Result::default()
        },
        depends_on: Vec::new(),
        invalidated_by: None,
        agent_session_id: None,
        agent_session_ids: Vec::new(),
        source_fingerprint: None,
        source_task_graph: None,
        completed_ids: Vec::new(),
        scaffold_hash: None,
        completion_evidence: Vec::new(),
        evidence_snapshot_hash: None,
        dispatched_items: Vec::new(),
        answered_by: None,
    }
}

fn none(_: &WorkflowV2CallRecord) -> bool {
    false
}

/// The R8 shape: round 1 refused twice (two logic versions), round 2
/// refused, round 3 accepted, then round 1 asked again under a third version
/// and published over round 3.
fn r8_shape() -> Vec<WorkflowV2CallRecord> {
    vec![
        landing("r1-v8", "t01", "TASK-B", "round-1", true, true),
        landing("r1-v9", "t02", "TASK-B", "round-1", true, true),
        landing("r2", "t03", "TASK-B", "round-2", true, true),
        landing("r3", "t04", "TASK-B", "round-3", true, false),
        landing("r1-v13", "t05", "TASK-B", "round-1", true, true),
    ]
}

#[test]
fn an_old_round_asked_again_after_a_newer_body_was_accepted_is_a_rerun() {
    let records = r8_shape();
    let lineage = LandingLineage::of(&records, "land", "TASK-B");
    let reruns: Vec<&str> = lineage
        .answers()
        .iter()
        .filter(|r| lineage.is_rerun(r))
        .map(|r| r.call.id.as_str())
        .collect();
    assert_eq!(reruns, vec!["r1-v13"]);
    assert_eq!(lineage.current().map(|r| r.call.id.as_str()), Some("r3"));
}

#[test]
fn a_superseded_round_replays_the_answer_its_next_round_consumed() {
    let records = r8_shape();
    let lineage = LandingLineage::of(&records, "land", "TASK-B");
    // Round 1 under a new id: the v9 answer, the one round 2 was authored
    // from; never the rerun, never a re-gate.
    let answer = lineage.superseded_answer(&request("TASK-B", "round-1"), 1, none);
    assert_eq!(answer.map(|r| r.call.id.as_str()), Some("r1-v9"));
    let answer = lineage.superseded_answer(&request("TASK-B", "round-2"), 1, none);
    assert_eq!(answer.map(|r| r.call.id.as_str()), Some("r2"));
    // The current body's request is the one request that may be judged again.
    assert!(
        lineage
            .superseded_answer(&request("TASK-B", "round-3"), 1, none)
            .is_none()
    );
    // A request never asked is new work.
    assert!(
        lineage
            .superseded_answer(&request("TASK-B", "round-4"), 1, none)
            .is_none()
    );
}

#[test]
fn an_answer_this_session_wrote_is_not_history() {
    let records = r8_shape();
    let lineage = LandingLineage::of(&records, "land", "TASK-B");
    let skip = |record: &WorkflowV2CallRecord| record.call.id.starts_with("r1");
    assert!(
        lineage
            .superseded_answer(&request("TASK-B", "round-1"), 1, skip)
            .is_none()
    );
}

/// An old round accepted again when no newer body had been accepted is not
/// a rerun: a refused round is no publication the old one overwrote.
#[test]
fn re_asking_over_refused_rounds_only_is_not_a_rerun() {
    let records = vec![
        landing("a1", "t01", "TASK-A", "round-1", true, true),
        landing("a2", "t02", "TASK-A", "round-2", true, true),
        landing("a1-v9", "t03", "TASK-A", "round-1", true, false),
    ];
    let lineage = LandingLineage::of(&records, "land", "TASK-A");
    assert!(lineage.answers().iter().all(|r| !lineage.is_rerun(r)));
    assert_eq!(lineage.current().map(|r| r.call.id.as_str()), Some("a1-v9"));
    assert!(
        lineage
            .superseded_answer(&request("TASK-A", "round-1"), 1, none)
            .is_none()
    );
    // Round 2 is history: it replays its own answer.
    let answer = lineage.superseded_answer(&request("TASK-A", "round-2"), 1, none);
    assert_eq!(answer.map(|r| r.call.id.as_str()), Some("a2"));
}

#[test]
fn interrupted_invalidated_and_other_subjects_are_not_answers() {
    let mut cut = landing("b1", "t01", "TASK-B", "round-1", false, true);
    cut.result.data["interrupted"] = serde_json::json!("paused");
    let mut stale = landing("b2", "t02", "TASK-B", "round-1", true, true);
    stale.invalidated_by = Some("restart".into());
    let other = landing("c1", "t03", "TASK-C", "round-1", true, true);
    let records = vec![cut, stale, other];
    let lineage = LandingLineage::of(&records, "land", "TASK-B");
    assert!(lineage.answers().is_empty());
    assert!(
        lineage
            .superseded_answer(&request("TASK-B", "round-1"), 1, none)
            .is_none()
    );
}

#[test]
fn a_refused_or_unpublished_landing_is_never_the_current_body() {
    let records = vec![
        landing("d1", "t01", "TASK-D", "round-1", true, false),
        landing("d2", "t02", "TASK-D", "round-2", true, true),
        landing("d3", "t03", "TASK-D", "round-3", false, false),
    ];
    let lineage = LandingLineage::of(&records, "land", "TASK-D");
    assert_eq!(lineage.current().map(|r| r.call.id.as_str()), Some("d1"));
    // The latest refused round replays its own answer.
    let answer = lineage.superseded_answer(&request("TASK-D", "round-2"), 1, none);
    assert_eq!(answer.map(|r| r.call.id.as_str()), Some("d2"));
}

/// Within one script execution an identical candidate evaluated twice (an
/// operational failure, then the same body again) is two evaluations: each
/// replays its own answer, in order. The same evaluation asked again under
/// another key is one, and replays the answer the script went on from.
#[test]
fn repeated_identical_candidates_replay_each_evaluation_in_order() {
    let mut operational = landing("e1", "t01", "TASK-E", "round-1", false, true);
    operational.result.data["gateEnvelope"]["operational_error"] =
        serde_json::json!({"text": "outage"});
    let mut repeat = landing("e1:occurrence:2", "t02", "TASK-E", "round-1", true, true);
    repeat
        .call
        .options
        .extra
        .insert(OCCURRENCE_KEY.into(), serde_json::json!(2));
    let next = landing("e2", "t03", "TASK-E", "round-2", true, false);
    let records = vec![operational, repeat, next];
    let lineage = LandingLineage::of(&records, "land", "TASK-E");
    let first = lineage.superseded_answer(&request("TASK-E", "round-1"), 1, none);
    assert_eq!(first.map(|r| r.call.id.as_str()), Some("e1"));
    let second = lineage.superseded_answer(&request("TASK-E", "round-1"), 2, none);
    assert_eq!(second.map(|r| r.call.id.as_str()), Some("e1:occurrence:2"));
    assert!(lineage.answers().iter().all(|r| !lineage.is_rerun(r)));
}

/// An identical body landed again as a later, distinct evaluation after
/// another body was accepted is a real round, not a rerun.
#[test]
fn a_later_identical_evaluation_is_a_round_of_its_own() {
    let mut again = landing("f1:occurrence:2", "t03", "TASK-F", "round-1", true, false);
    again
        .call
        .options
        .extra
        .insert(OCCURRENCE_KEY.into(), serde_json::json!(2));
    let records = vec![
        landing("f1", "t01", "TASK-F", "round-1", true, true),
        landing("f2", "t02", "TASK-F", "round-2", true, false),
        again,
    ];
    let lineage = LandingLineage::of(&records, "land", "TASK-F");
    assert!(lineage.answers().iter().all(|r| !lineage.is_rerun(r)));
    assert_eq!(
        lineage.current().map(|r| r.call.id.as_str()),
        Some("f1:occurrence:2")
    );
}
