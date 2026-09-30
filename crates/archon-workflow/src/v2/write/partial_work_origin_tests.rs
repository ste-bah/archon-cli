//! The attempt origin a resumed branch is told about.

use super::*;

fn rejected_origin() -> PartialOrigin {
    PartialOrigin::from_result(&WorkflowV2Result {
        status: WorkflowV2Status::NeedsReview,
        summary: "repository audit rejected unexplained or unauthorized changes: tests/extra.rs."
            .into(),
        residual_gaps: vec![
            crate::v2::WorkflowV2ResidualGap {
                id: "audit_unexplained_change".into(),
                description: "tests/extra.rs changed without an audit disposition".into(),
                severity: Some("blocker".into()),
            },
            crate::v2::WorkflowV2ResidualGap {
                id: "coverage_missing".into(),
                description: "no test covers the new branch".into(),
                severity: None,
            },
        ],
        ..Default::default()
    })
}

/// Issue-20, the live failure: a coder rejected by a gate was re-dispatched
/// with "ran out of time" and repeated the identical omission. The resume
/// preamble must carry the verdict: status, summary, and every gap.
#[test]
fn a_rejection_origin_names_the_status_summary_and_every_gap() {
    let partial = PartialWork {
        patch_path: "p".into(),
        files: vec!["a.rs".into()],
        bytes: 1,
        baseline_commit: "c".into(),
        origin: Some(rejected_origin()),
    };
    let text = with_host_preamble("do the task", None, Some(&partial), &Default::default());
    assert!(
        text.starts_with(
            "A previous attempt at this task was not accepted (status: needs_review): repository audit rejected unexplained or unauthorized changes: tests/extra.rs.\nIts unresolved gaps, which you must fix:\n- [blocker] audit_unexplained_change: tests/extra.rs changed without an audit disposition\n- [gap] coverage_missing: no test covers the new branch\nIts uncommitted work (1 file(s)) has been applied to this workspace: a.rs. Continue from that work; do not start over, and do not discard it unless it is wrong."
        ),
        "{text}"
    );
    assert!(!text.contains("ran out of time"));
    assert!(text.ends_with("\n\ndo the task"));
    // A restart in the same worktree keeps its own opening but is still told
    // the gaps the verdict named.
    let restarted = with_restart_preamble("do the task", None, Some(&partial), &Default::default());
    assert!(
        restarted.starts_with("This is the same attempt, restarted"),
        "{restarted}"
    );
    assert!(restarted.contains("- [blocker] audit_unexplained_change:"));
    assert!(!restarted.contains("was not accepted"));
}

/// A host cut is not a verdict: the sentence stays, and the timeout gap the
/// interruption result carries is not presented as something to fix.
#[test]
fn a_timeout_origin_keeps_the_ran_out_of_time_sentence() {
    let origin = PartialOrigin::from_result(
        &super::super::super::errors::write_branch_interrupted_result(
            "agents-2-0",
            &serde_json::json!({"item": {"canonical_task_ids": ["TASK-001"]}}),
            "host call timed out after 7200s",
        ),
    );
    assert!(origin.is_timeout(), "{origin:?}");
    assert!(!rejected_origin().is_timeout());
    // An agent's own verdict that mentions a timeout is still a verdict.
    let judged = PartialOrigin::from_result(&WorkflowV2Result {
        status: WorkflowV2Status::Failed,
        summary: "integration tests timed out; the retry loop never terminates".into(),
        ..Default::default()
    });
    assert!(!judged.is_timeout(), "{judged:?}");
    let partial = PartialWork {
        patch_path: "p".into(),
        files: vec!["a.rs".into()],
        bytes: 1,
        baseline_commit: "c".into(),
        origin: Some(origin),
    };
    let text = with_host_preamble("do the task", None, Some(&partial), &Default::default());
    assert!(text.starts_with("A previous attempt at this task ran out of time before finishing. Its uncommitted work (1 file(s))"), "{text}");
    assert!(!text.contains("unresolved gaps") && !text.contains("write_branch_timeout"));
}

/// The sidecar beside the patch flattens the partial, so the origin rides
/// along and comes back whole; a sidecar an older binary wrote has none and
/// still deserializes.
#[test]
fn the_sidecar_round_trips_the_origin_and_reads_without_one() {
    let temp = tempfile::tempdir().unwrap();
    let (_canonical, first, _second) = repo_with_worktrees(temp.path());
    std::fs::write(first.join("lib.rs"), "fn a() {}\nfn b() {}\n").unwrap();
    let run_root = temp.path().join("run");
    let partial = capture_partial_work(
        &first,
        &run_root,
        "agents-2",
        "agents-2-0",
        &["TASK-001".to_string()],
        Some(rejected_origin()),
    )
    .unwrap()
    .unwrap();
    let sidecar_path = super::super::super::partial_work_lookup::sidecar_path(&partial.patch_path);
    let raw = std::fs::read_to_string(&sidecar_path).unwrap();
    assert!(
        raw.contains("\"origin\"") && raw.contains("audit_unexplained_change"),
        "{raw}"
    );
    let sidecar: super::super::super::partial_work_lookup::PartialSidecar =
        serde_json::from_str(&raw).unwrap();
    assert_eq!(sidecar.partial, partial);
    assert_eq!(sidecar.partial.origin, Some(rejected_origin()));
    let store = WorkflowV2ResultStore::new(run_root.join("v2"));
    let found =
        latest_partial_for_tasks(&store, &["TASK-001".to_string()]).expect("found via sidecar");
    assert_eq!(found.origin, Some(rejected_origin()));
    // The pre-Issue-20 shape: no `origin` key at all.
    let legacy = format!(
        r#"{{"schema_version":1,"stage_id":"s","branch_id":"i","canonical_task_ids":["TASK-001"],"captured_at":"2026-09-15T00:00:00Z","patch_path":{},"files":["lib.rs"],"bytes":3,"baseline_commit":"c"}}"#,
        serde_json::to_string(&partial.patch_path).unwrap()
    );
    let parsed: super::super::super::partial_work_lookup::PartialSidecar =
        serde_json::from_str(&legacy).unwrap();
    assert_eq!(parsed.partial.origin, None);
    assert_eq!(parsed.partial.files, vec!["lib.rs".to_string()]);
    let legacy_partial: PartialWork =
        serde_json::from_str(r#"{"patch_path":"p","files":[],"bytes":0,"baseline_commit":"c"}"#)
            .unwrap();
    assert_eq!(legacy_partial.origin, None);
}

/// Batch O: the origin keeps its summary and EVERY gap whole, however many
/// and however long, so a resumed attempt is told to fix all of them.
#[test]
fn the_origin_keeps_every_gap_and_its_summary_whole() {
    let result = WorkflowV2Result {
        status: WorkflowV2Status::Failed,
        summary: "s".repeat(5000),
        residual_gaps: (0..30)
            .map(|i| crate::v2::WorkflowV2ResidualGap {
                id: format!("g{i}"),
                description: "d".repeat(3000),
                severity: None,
            })
            .collect(),
        ..Default::default()
    };
    let origin = PartialOrigin::from_result(&result);
    assert_eq!(origin.status, "failed");
    assert_eq!(origin.summary.chars().count(), 5000);
    assert_eq!(origin.residual_gaps.len(), 30);
    assert!(
        origin
            .residual_gaps
            .iter()
            .all(|gap| gap.description.chars().count() == 3000)
    );
    assert_eq!(origin.residual_gaps[29].id, "g29");
}
