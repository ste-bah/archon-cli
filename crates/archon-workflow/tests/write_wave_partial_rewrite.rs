//! Issue-18: a later placeholder outcome must not hide captured partial work.
//!
//! Live on wf-719ff3b0 `agents-5-0`: the branch failed with a 22-file partial
//! recorded on its outcome; a resume pass then re-recorded the item as "was
//! not dispatched: its dependencies have no accepted outcome", which moved the
//! record with the partial into `superseded/`. The lookup read the current
//! record only, so the re-dispatched session got a clean worktree and no
//! resume preamble. This drives the production write-wave seam through that
//! sequence: a wave fails with partial work, the outcome is rewritten by a
//! bare placeholder (raw store write, as the old binary left it), and the next
//! wave for the same task still applies the partial and says so.
#[path = "support/write_wave_fixture.rs"]
mod support;

use archon_workflow::*;
use serde_json::json;
use support::{Edits, Fixture, git};

#[tokio::test]
async fn a_placeholder_rewrite_does_not_hide_the_partial_from_the_next_wave() {
    let f = Fixture::new();
    let (out, _) = f
        .wave_scripted(
            "first",
            vec![(
                vec!["owned.txt"],
                Edits {
                    files: vec![("owned.txt", "half done\n")],
                    report: vec![],
                    via_adapter: false,
                },
            )],
            None,
            &["first-0"],
        )
        .await;
    assert_ne!(out.status, WorkflowV2Status::Accepted, "{out:#?}");
    assert_eq!(
        git(&f.repo, &["rev-parse", "HEAD"]),
        f.base,
        "nothing landed"
    );
    let failed = f.branch_result("first", "first-0");
    let patch_path = failed.data["partial_work"]["patch_path"]
        .as_str()
        .expect("partial captured on the failed outcome")
        .to_string();
    assert!(std::path::Path::new(&patch_path).is_file());
    let sidecar = std::path::Path::new(&patch_path).with_file_name("first-0.partial.json");
    let sidecar: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&sidecar).expect("sidecar beside patch"))
            .unwrap();
    assert_eq!(sidecar["canonical_task_ids"], json!(["TASK-001"]));
    assert_eq!(sidecar["files"], json!(["owned.txt"]));

    // The old binary's resume pass: the item re-recorded as a bare placeholder
    // under a new input hash, the failed record archived under `superseded/`.
    let placeholder = WorkflowV2BranchOutcome {
        item_id: "first-0".into(),
        role: "coder".into(),
        status: WorkflowV2Status::NeedsReview,
        result: Some(WorkflowV2Result {
            status: WorkflowV2Status::NeedsReview,
            summary: "write branch 'first-0' was not dispatched: its dependencies TASK-000 have no accepted outcome in this run".into(),
            data: json!({"branch_id": "first-0", "item_id": "first-0",
                "canonical_task_ids": ["TASK-001"], "blocked_on_dependency": ["TASK-000"]}),
            ..Default::default()
        }),
        error: None,
        failure_kind: None,
        item_input_hash: Some("resume-pass".into()),
        completion_evidence: vec![],
    };
    f.v2.save_branch_outcome("first", &placeholder).unwrap();
    let current = f.branch_result("first", "first-0");
    assert!(
        current.data.get("partial_work").is_none(),
        "placeholder is bare"
    );
    let superseded = f.v2.root().join("branches/first/superseded");
    assert_eq!(std::fs::read_dir(&superseded).unwrap().count(), 1);

    // Re-dispatch for the same task: the partial is applied and the agent told.
    let (out, prompts) = f
        .wave_scripted(
            "second",
            vec![(
                vec!["owned.txt"],
                Edits {
                    files: vec![],
                    report: vec!["owned.txt"],
                    via_adapter: false,
                },
            )],
            None,
            &[],
        )
        .await;
    assert_eq!(out.status, WorkflowV2Status::Accepted, "{out:#?}");
    assert_eq!(prompts.len(), 1);
    assert!(
        prompts[0]
            .contains("uncommitted work (1 file(s)) has been applied to this workspace: owned.txt"),
        "{}",
        prompts[0]
    );
    // Issue-20: the branch ended `failed` on its own verdict, not on a host
    // cut, so the re-dispatch is told that verdict rather than "ran out of
    // time".
    assert!(
        prompts[0].contains(
            "A previous attempt at this task was not accepted (status: failed): scripted: ran out of budget after writing."
        ),
        "{}",
        prompts[0]
    );
    assert!(!prompts[0].contains("ran out of time"), "{}", prompts[0]);
    assert_eq!(
        git(&f.repo, &["show", "HEAD:owned.txt"]),
        "half done",
        "the partial landed"
    );
}

/// The same sequence through the host's own writer: a placeholder recorded
/// via the write funnel keeps the partial on the CURRENT record too, so a
/// reader of the current record alone (status, the operator) still sees it.
#[tokio::test]
async fn a_rewrite_through_the_write_funnel_keeps_the_partial_on_the_current_record() {
    let f = Fixture::new();
    let (out, _) = f
        .wave_scripted(
            "first",
            vec![(
                vec!["owned.txt"],
                Edits {
                    files: vec![("owned.txt", "half done\n")],
                    report: vec![],
                    via_adapter: false,
                },
            )],
            None,
            &["first-0"],
        )
        .await;
    assert_ne!(out.status, WorkflowV2Status::Accepted, "{out:#?}");
    let partial = f.branch_result("first", "first-0").data["partial_work"].clone();
    assert!(partial["patch_path"].is_string());
    // A second pass over the same call id with the branch failing again but
    // this time writing nothing new: the wave re-applies the partial into the
    // worktree, re-captures it, and the current record still names it.
    let (out, prompts) = f
        .wave_scripted(
            "first",
            vec![(
                vec!["owned.txt"],
                Edits {
                    files: vec![],
                    report: vec![],
                    via_adapter: false,
                },
            )],
            None,
            &["first-0"],
        )
        .await;
    assert_ne!(out.status, WorkflowV2Status::Accepted, "{out:#?}");
    assert!(
        prompts[0].contains("uncommitted work (1 file(s)) has been applied"),
        "{}",
        prompts[0]
    );
    let again = f.branch_result("first", "first-0").data["partial_work"].clone();
    assert_eq!(again["patch_path"], partial["patch_path"]);
    assert_eq!(again["files"], json!(["owned.txt"]));
}
