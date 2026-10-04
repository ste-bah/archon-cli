//! Issue 263, round 3 (decision B): a write session that edits files and is
//! then stopped by the runner for making no progress keeps its edits. The
//! wave captures them as partial work and persists the branch outcome with
//! the stall diagnosis, so remediation proceeds; the run is never paused at
//! the branch. A later wave (the resume) applies the edits and lands them.
#[path = "support/write_wave_fixture.rs"]
mod support;

use archon_workflow::*;
use support::{Edits, Fixture, STALL, git};

#[tokio::test]
async fn a_stalled_session_keeps_its_edits_and_goes_to_remediation() {
    let f = Fixture::new();
    let (out, _) = f
        .wave_scripted(
            "first",
            vec![(
                vec!["owned.txt"],
                Edits {
                    files: vec![("owned.txt", "half done\n"), ("owned.txt", STALL)],
                    report: vec![],
                    via_adapter: false,
                },
            )],
            None,
            &[],
        )
        .await;
    assert_ne!(out.status, WorkflowV2Status::Accepted, "{out:#?}");
    assert_ne!(
        f.store.load_state(&f.run).unwrap().status,
        RunStatus::Paused,
        "a branch stall never pauses the run"
    );
    let stalled = f.branch_result("first", "first-0");
    assert_eq!(
        stalled.data["branch_no_progress_stop"], true,
        "{stalled:#?}"
    );
    let patch = stalled.data["partial_work"]["patch_path"]
        .as_str()
        .expect("the edits were captured as partial work");
    assert!(std::path::Path::new(patch).is_file());
    assert!(
        !out.residual_gaps.is_empty(),
        "the stall reaches remediation: {out:#?}"
    );

    // The resumed wave for the same task: the edits come back and land.
    let (out, prompts) = f
        .wave_scripted(
            "first",
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
    assert!(
        prompts[0].contains("uncommitted work (1 file(s)) has been applied"),
        "{}",
        prompts[0]
    );
    assert_eq!(git(&f.repo, &["show", "HEAD:owned.txt"]), "half done");
}
