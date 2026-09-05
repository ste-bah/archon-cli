use super::*;
use crate::v2::host_command::HostCommandRequest;
use crate::{WorkflowV2HostCall, WorkflowV2HostOptions, WorkflowV2Result, WorkflowV2Status};

fn record(
    id: &str,
    method: WorkflowV2HostMethod,
    started_at: &str,
    command: Option<&str>,
    tasks: &[&str],
) -> WorkflowV2CallRecord {
    let mut options = WorkflowV2HostOptions::default();
    if let Some(command_id) = command {
        options.host_command = Some(HostCommandRequest {
            command_id: command_id.to_string(),
            stdin: None,
        });
    }
    let mut result = WorkflowV2Result::default();
    result.data = serde_json::json!({
        "subjects": tasks.iter().map(|t| serde_json::json!({"taskId": t, "fileName": format!("{t}.md")})).collect::<Vec<_>>()
    });
    WorkflowV2CallRecord {
        run_id: "wf-history".into(),
        call: WorkflowV2HostCall {
            id: id.into(),
            method,
            write_mode: None,
            options,
        },
        attempt: 1,
        schema_version: "1".into(),
        started_at: started_at.into(),
        finished_at: started_at.into(),
        input_hash: "in".into(),
        output_hash: "out".into(),
        status: WorkflowV2Status::NeedsReview,
        result,
        depends_on: Vec::new(),
        invalidated_by: None,
        agent_session_id: None,
        source_fingerprint: None,
        source_task_graph: None,
        completed_ids: Vec::new(),
        scaffold_hash: None,
        completion_evidence: Vec::new(),
        evidence_snapshot_hash: None,
    }
}

#[test]
fn a_family_is_the_id_without_its_trailing_ordinal() {
    assert_eq!(
        call_family("acceptance-author-3"),
        ("acceptance-author", Some(3))
    );
    assert_eq!(
        call_family("fixed-decomposition-final"),
        ("fixed-decomposition-final", None)
    );
    assert_eq!(call_family("x-"), ("x-", None));
}

/// Earlier attempts of a phase are history once a later attempt exists; the
/// latest attempt, and any record of another family, are not.
#[test]
fn an_agent_record_is_superseded_only_by_a_later_ordinal_of_its_family() {
    let a1 = record(
        "acceptance-author-1",
        WorkflowV2HostMethod::Agent,
        "t1",
        None,
        &[],
    );
    let a2 = record(
        "acceptance-author-2",
        WorkflowV2HostMethod::Agent,
        "t2",
        None,
        &[],
    );
    let s1 = record(
        "skeleton-author-1",
        WorkflowV2HostMethod::Agent,
        "t3",
        None,
        &[],
    );
    let all = vec![a1.clone(), a2.clone(), s1.clone()];
    assert!(superseded(&a1, &all));
    assert!(!superseded(&a2, &all));
    assert!(!superseded(&s1, &all));
    let mut invalidated = all.clone();
    invalidated[1].invalidated_by = Some("restart".into());
    assert!(
        !superseded(&a1, &invalidated),
        "an invalidated successor supersedes nothing"
    );
}

/// A freeze is history once the same command has run again later over the
/// same subject; a later run over ANOTHER subject retires nothing, so the last
/// landing of each task keeps its live checks.
#[test]
fn a_host_record_is_superseded_only_by_a_later_run_over_the_same_subject() {
    let acc1 = record(
        "h-1",
        WorkflowV2HostMethod::HostCommand,
        "2026-09-05T01:00:00Z",
        Some("freeze-a"),
        &[],
    );
    let acc2 = record(
        "h-2",
        WorkflowV2HostMethod::HostCommand,
        "2026-09-05T01:10:00Z",
        Some("freeze-a"),
        &[],
    );
    let body_a1 = record(
        "h-3",
        WorkflowV2HostMethod::HostCommand,
        "2026-09-05T01:20:00Z",
        Some("land"),
        &["T-A"],
    );
    let body_b1 = record(
        "h-4",
        WorkflowV2HostMethod::HostCommand,
        "2026-09-05T01:30:00Z",
        Some("land"),
        &["T-B"],
    );
    let body_a2 = record(
        "h-5",
        WorkflowV2HostMethod::HostCommand,
        "2026-09-05T01:40:00Z",
        Some("land"),
        &["T-A"],
    );
    let all = vec![
        acc1.clone(),
        acc2.clone(),
        body_a1.clone(),
        body_b1.clone(),
        body_a2.clone(),
    ];
    assert!(
        superseded(&acc1, &all),
        "the earlier contract freeze is history"
    );
    assert!(!superseded(&acc2, &all), "the last contract freeze is live");
    assert!(
        superseded(&body_a1, &all),
        "task A's first landing is history"
    );
    assert!(
        !superseded(&body_b1, &all),
        "task B's only landing is live: task A's later landing must not retire it"
    );
    assert!(!superseded(&body_a2, &all));
}

/// Verbatim replay needs the recorded input and an uninvalidated record.
#[test]
fn replay_requires_the_recorded_input_and_an_uninvalidated_record() {
    let a1 = record(
        "acceptance-author-1",
        WorkflowV2HostMethod::Agent,
        "t1",
        None,
        &[],
    );
    let a2 = record(
        "acceptance-author-2",
        WorkflowV2HostMethod::Agent,
        "t2",
        None,
        &[],
    );
    let all = vec![a1.clone(), a2.clone()];
    assert!(replayable_history(&a1, &all, "in"));
    assert!(!replayable_history(&a1, &all, "other-input"));
    let mut dead = a1.clone();
    dead.invalidated_by = Some("restart".into());
    assert!(!replayable_history(&dead, &all, "in"));
}

/// A refused candidate landed nothing, so it never retires the landing it
/// followed; a later landing does.
#[test]
fn a_refusal_never_supersedes_a_landed_freeze() {
    let mut landed = record(
        "h-1",
        WorkflowV2HostMethod::HostCommand,
        "2026-09-05T01:00:00Z",
        Some("freeze-a"),
        &[],
    );
    landed.result.data["publicationReceipt"] = serde_json::json!({"call_id": "h-1"});
    let refused = record(
        "h-2",
        WorkflowV2HostMethod::HostCommand,
        "2026-09-05T01:10:00Z",
        Some("freeze-a"),
        &[],
    );
    let mut relanded = record(
        "h-3",
        WorkflowV2HostMethod::HostCommand,
        "2026-09-05T01:20:00Z",
        Some("freeze-a"),
        &[],
    );
    relanded.result.data["publicationReceipt"] = serde_json::json!({"call_id": "h-3"});
    assert!(
        !superseded(&landed, &[landed.clone(), refused.clone()]),
        "a refusal retires nothing landed"
    );
    assert!(
        superseded(&refused, &[refused.clone(), relanded.clone()]),
        "a refusal is history once anything follows it"
    );
    assert!(
        superseded(
            &landed,
            &[landed.clone(), refused.clone(), relanded.clone()]
        ),
        "a later landing does retire it"
    );
}
