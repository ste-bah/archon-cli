//! Issue-24: a committed write branch is reused on resume even after the tree
//! it was measured against has moved.
//!
//! Live: the host stamps each declared target's current line count into the
//! item before asking which stored outcomes it may reuse, and the stored hash
//! was taken from that stamped input. When a later wave grew a file an earlier
//! wave's item also declared, the earlier item's hash moved on the next
//! resume, reuse was refused, and the already-committed task was dispatched to
//! a coder again — 20 to 60 minutes per item, on every resume — where it stalled
//! at the read wall with nothing left to write.
//!
//! Drives the production write-wave seam end to end: one wave lands, the
//! canonical tree grows the same file, the same call is re-run. The fixture's
//! prompt log is the dispatch count.
#[path = "support/write_wave_fixture.rs"]
mod support;

use archon_workflow::*;
use serde_json::json;
use support::{Edits, Fixture, git};

const CALL: &str = "implementation-wave-1";
const BRANCH: &str = "implementation-wave-1-0";

fn edits() -> Edits {
    Edits {
        files: vec![("owned.txt", "implemented\n")],
        report: vec!["owned.txt"],
        via_adapter: false,
    }
}

/// One wave that lands `owned.txt`; the commit it produced.
async fn land_first_wave(f: &Fixture) -> String {
    let (first, prompts) = f
        .wave_audited(CALL, vec![(vec!["owned.txt"], edits())], None)
        .await;
    assert_eq!(first.status, WorkflowV2Status::Accepted, "{first:#?}");
    assert_eq!(prompts.len(), 1, "the first wave dispatches its one coder");
    let landed = git(&f.repo, &["rev-parse", "HEAD"]);
    assert_ne!(landed, f.base, "the wave committed");
    assert_eq!(
        f.manifest(CALL, BRANCH)["status"]["status"],
        json!("applied"),
        "the host's apply receipt"
    );
    assert_eq!(
        f.branch_result(CALL, BRANCH).data["patch_landed"],
        json!(true),
        "the branch's own landed marker"
    );
    landed
}

/// A later wave grows the file the first wave's item declared, exactly the
/// tree movement that used to change the earlier item's hash.
fn later_wave_grows_the_target(f: &Fixture) {
    std::fs::write(f.repo.join("owned.txt"), "implemented\n".repeat(40)).unwrap();
    git(&f.repo, &["commit", "-qam", "a later wave grew owned.txt"]);
}

#[tokio::test]
async fn a_landed_branch_is_reused_after_a_later_wave_grows_its_target_file() {
    let f = Fixture::new();
    land_first_wave(&f).await;
    later_wave_grows_the_target(&f);
    let grown = git(&f.repo, &["rev-parse", "HEAD"]);

    // Resume: the same call, the same authored item, a different line budget.
    let (resumed, prompts) = f
        .wave_audited(CALL, vec![(vec!["owned.txt"], edits())], None)
        .await;

    assert!(
        prompts.is_empty(),
        "the committed branch was dispatched again: {}",
        prompts.len()
    );
    assert_eq!(resumed.status, WorkflowV2Status::Accepted, "{resumed:#?}");
    assert_eq!(
        git(&f.repo, &["rev-parse", "HEAD"]),
        grown,
        "a reused branch writes nothing"
    );
    assert_eq!(
        std::fs::read_to_string(f.repo.join("owned.txt")).unwrap(),
        "implemented\n".repeat(40),
        "the later wave's growth is untouched"
    );
}

/// The landed short-circuit on the live path: the item is re-authored (a
/// wider declared scope), so its authored identity differs too — and the
/// branch is still reused, because its patch was applied and its task is
/// recorded landed in this run. Nothing about re-running a committed task can
/// be right.
#[tokio::test]
async fn a_landed_branch_is_reused_even_when_the_item_was_reauthored() {
    let f = Fixture::new();
    land_first_wave(&f).await;
    later_wave_grows_the_target(&f);
    let grown = git(&f.repo, &["rev-parse", "HEAD"]);

    let (resumed, prompts) = f
        .wave_audited(CALL, vec![(vec!["owned.txt", "other.txt"], edits())], None)
        .await;

    assert!(
        prompts.is_empty(),
        "the committed branch was dispatched again: {}",
        prompts.len()
    );
    assert_eq!(resumed.status, WorkflowV2Status::Accepted, "{resumed:#?}");
    assert_eq!(git(&f.repo, &["rev-parse", "HEAD"]), grown);
}
