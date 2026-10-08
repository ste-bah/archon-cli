//! Issue-292: every reader of the branch store and the call archive opens
//! only regular files, or links resolving to regular files inside their own
//! directory, within a size bound. A FIFO, a link to a FIFO, a link out of
//! the directory, an oversized file or a directory is skipped unread, with
//! a warning that names it, and never counts as an outcome. Each read runs
//! under [`store_entries::bounded`], so a regression fails instead of
//! hanging the suite.
#![cfg(unix)]

#[path = "support/branch_revocation.rs"]
mod branch_revocation;
#[path = "support/restart_run.rs"]
mod restart_run;
#[path = "support/store_entries.rs"]
mod store_entries;

use std::path::{Path, PathBuf};

use archon_workflow::v2::restart::restart_generated_v2_task;
use archon_workflow::{WorkflowV2ResultStore, WorkflowV2Status};
use branch_revocation::*;
use restart_run::{accepted, agent_call, generated_run, interrupted, v2_store};
use store_entries::{bounded, mkfifo, write_oversized};

/// A run whose `T-A` landed and was superseded, and the archive directory.
fn archived_run() -> (tempfile::TempDir, WorkflowV2ResultStore, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let (store, run) = generated_run(&temp, &[CALL]);
    let v2 = v2_store(&store, &run);
    landed_then_superseded(&v2, "T-A");
    let archive = call_dir(&v2).join("superseded");
    (temp, v2, archive)
}

fn call_dir(v2: &WorkflowV2ResultStore) -> PathBuf {
    let current = v2.branch_outcome_path(CALL, &item("T-A").id);
    current.parent().unwrap().to_path_buf()
}

/// A landed outcome of `T-B` as bytes: reusable wherever a reader takes it.
fn landed_b() -> Vec<u8> {
    serde_json::to_vec(&outcome("T-B", WorkflowV2Status::Accepted, "H1", true)).unwrap()
}

/// Every branch reader over the archive: the superseded outcomes, and one
/// split of `T-A` and `T-B` (the landing-record reader).
fn read_branches(v2: &WorkflowV2ResultStore) -> (usize, Vec<String>) {
    let superseded = v2.load_superseded_branch_outcomes().len();
    let (reused, _) = split(v2, &["T-A", "T-B"]);
    (superseded, reused)
}

/// Run the branch readers with `planted` in place and check that none
/// blocked, none counted it, and each skip named it.
fn assert_skipped(v2: WorkflowV2ResultStore, planted: &Path, fifos: &[PathBuf]) {
    let reader = v2.clone();
    let ((superseded, reused), warnings) = bounded(fifos, move || read_branches(&reader));
    assert_eq!(superseded, 1, "only T-A's archived record counts");
    assert_eq!(
        reused,
        vec![item("T-A").id],
        "the planted entry was reused as an outcome"
    );
    assert!(
        warnings.name(planted),
        "{planted:?} was skipped silently: {:?}",
        warnings.lines()
    );
}

#[test]
fn a_fifo_in_the_archive_is_skipped_unread_and_reported() {
    let (_temp, v2, archive) = archived_run();
    let fifo = archive.join("planted.json");
    mkfifo(&fifo);
    assert_skipped(v2, &fifo, &[fifo.clone()]);
}

#[test]
fn a_link_to_a_fifo_in_the_archive_is_skipped_unread_and_reported() {
    let (_temp, v2, archive) = archived_run();
    let fifo = archive.join("pipe");
    mkfifo(&fifo);
    let link = archive.join("linked.json");
    std::os::unix::fs::symlink(&fifo, &link).unwrap();
    assert_skipped(v2, &link, &[fifo.clone()]);
}

#[test]
fn a_link_out_of_the_archive_is_never_an_outcome() {
    let (temp, v2, archive) = archived_run();
    let outside = temp.path().join("landing-b.json");
    std::fs::write(&outside, landed_b()).unwrap();
    let link = archive.join("outside.json");
    std::os::unix::fs::symlink(&outside, &link).unwrap();
    assert_skipped(v2, &link, &[]);
}

#[test]
fn an_oversized_archive_file_is_never_an_outcome() {
    let (_temp, v2, archive) = archived_run();
    let big = archive.join("big.json");
    write_oversized(&big, &landed_b());
    assert_skipped(v2, &big, &[]);
}

#[test]
fn a_directory_in_the_archive_is_skipped_and_reported() {
    let (_temp, v2, archive) = archived_run();
    let dir = archive.join("nested.json");
    std::fs::create_dir(&dir).unwrap();
    assert_skipped(v2, &dir, &[]);
}

#[test]
fn a_link_inside_the_archive_is_still_read() {
    let (_temp, v2, archive) = archived_run();
    let record = archive.join("landing-b.record");
    std::fs::write(&record, landed_b()).unwrap();
    std::os::unix::fs::symlink(&record, archive.join("linked-b.json")).unwrap();
    let reader = v2.clone();
    let ((superseded, mut reused), _) = bounded(&[], move || read_branches(&reader));
    reused.sort();
    assert_eq!(superseded, 2, "T-A's record and the in-archive link");
    assert_eq!(reused, vec![item("T-A").id, item("T-B").id]);
}

#[test]
fn a_fifo_current_slot_reads_as_no_outcome_and_is_reported() {
    let (_temp, v2, _) = archived_run();
    let slot = v2.branch_outcome_path(CALL, &item("T-B").id);
    mkfifo(&slot);
    let reader = v2.clone();
    let (loaded, warnings) = bounded(&[slot.clone()], move || {
        let one = reader.load_branch_outcome(CALL, &item("T-B").id).unwrap();
        let all = reader.load_branch_outcomes_for_call(CALL).unwrap().len();
        (one.is_none(), all)
    });
    assert_eq!(loaded, (true, 1), "only T-A's current outcome");
    assert!(warnings.name(&slot), "{:?}", warnings.lines());
    // A save over it never reads it, and moves it out of the slot.
    let saver = v2.clone();
    let (saved, _) = bounded(&[slot.clone()], move || {
        let outcome = outcome("T-B", WorkflowV2Status::Accepted, "H3", false);
        saver.save_branch_outcome(CALL, &outcome).map(|_| ())
    });
    saved.unwrap();
    assert!(std::fs::metadata(&slot).unwrap().is_file());
}

#[test]
fn a_fifo_in_a_calls_history_never_blocks_its_lookups() {
    let temp = tempfile::tempdir().unwrap();
    let (store, run) = generated_run(&temp, &[CALL]);
    let v2 = v2_store(&store, &run);
    v2.save_call_record(&accepted("call-x", "T-X")).unwrap();
    let history = v2.call_history_dir("call-x");
    std::fs::create_dir_all(&history).unwrap();
    let fifo = history.join("planted.json");
    mkfifo(&fifo);
    let reader = v2.clone();
    let (attempt, warnings) = bounded(&[fifo.clone()], move || reader.next_attempt("call-x"));
    attempt.unwrap();
    assert!(warnings.name(&fifo), "{:?}", warnings.lines());
}

#[test]
fn a_fifo_call_slot_is_quarantined_unread() {
    let temp = tempfile::tempdir().unwrap();
    let (store, run) = generated_run(&temp, &[CALL]);
    let v2 = v2_store(&store, &run);
    let slot = v2.result_path("call-y");
    std::fs::create_dir_all(slot.parent().unwrap()).unwrap();
    mkfifo(&slot);
    let reader = v2.clone();
    let (healed, _) = bounded(&[slot.clone()], move || {
        reader
            .load_call_slot_healing("call-y")
            .map(|slot| format!("{slot:?}"))
    });
    let healed = healed.unwrap();
    assert!(healed.starts_with("Damaged"), "{healed}");
    assert!(std::fs::symlink_metadata(&slot).is_err());
}

#[test]
fn a_link_from_the_archive_into_revoked_is_never_an_outcome() {
    let (_temp, v2, archive) = archived_run();
    let revoked = call_dir(&v2).join("revoked");
    std::fs::create_dir_all(&revoked).unwrap();
    std::fs::write(revoked.join("old-b.json"), landed_b()).unwrap();
    let link = archive.join("revived.json");
    std::os::unix::fs::symlink("../revoked/old-b.json", &link).unwrap();
    assert_skipped(v2, &link, &[]);
}

#[test]
fn a_linked_archive_directory_is_never_read() {
    let (temp, v2, archive) = archived_run();
    let elsewhere = temp.path().join("elsewhere");
    std::fs::create_dir(&elsewhere).unwrap();
    std::fs::write(elsewhere.join("landing-b.json"), landed_b()).unwrap();
    std::fs::rename(&archive, temp.path().join("moved-archive")).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &archive).unwrap();
    let reader = v2.clone();
    let ((superseded, reused), warnings) = bounded(&[], move || read_branches(&reader));
    assert_eq!(superseded, 0, "nothing read through the linked archive");
    assert!(!reused.contains(&item("T-B").id), "{reused:?}");
    assert!(
        warnings.name(&archive.join("landing-b.json")),
        "{:?}",
        warnings.lines()
    );
}

/// A link out of the archive is never read, so restart neither reuses nor
/// moves it, and the file it names is untouched.
#[test]
fn restart_never_reads_or_moves_a_link_out_of_the_archive() {
    let temp = tempfile::tempdir().unwrap();
    let (store, run) = generated_run(&temp, &[CALL]);
    let v2 = v2_store(&store, &run);
    landed_then_superseded(&v2, "T-A");
    let outside = temp.path().join("landing-a.json");
    let landed_a = outcome("T-A", WorkflowV2Status::Accepted, "H1", true);
    std::fs::write(&outside, serde_json::to_vec(&landed_a).unwrap()).unwrap();
    let link = call_dir(&v2).join("superseded/outside.json");
    std::os::unix::fs::symlink(&outside, &link).unwrap();
    let (restarted, warnings) = bounded(&[], move || {
        restart_generated_v2_task(&store, &run, "T-A").map_err(|e| e.to_string())
    });
    restarted.unwrap();
    assert!(warnings.name(&link), "{:?}", warnings.lines());
    assert_revoked(&v2, "T-A");
    assert!(
        std::fs::symlink_metadata(&link).is_ok(),
        "it was never planned"
    );
    assert_eq!(
        std::fs::read(&outside).unwrap(),
        serde_json::to_vec(&landed_a).unwrap()
    );
}

/// A call slot the readers refuse is a gap in the call's history: an older
/// archived answer never stands in for it (Issue-292, the Issue-313 rule).
#[test]
fn a_refused_call_slot_leaves_a_gap_no_archived_record_fills() {
    let temp = tempfile::tempdir().unwrap();
    let (store, run) = generated_run(&temp, &[CALL]);
    let v2 = v2_store(&store, &run);
    let outside = temp.path().join("slot-target.json");
    for (id, plant) in [("call-dir", 0), ("call-out", 1)] {
        v2.save_call_record(&accepted(id, "T-Z")).unwrap();
        v2.save_call_record(&interrupted(id)).unwrap();
        let input = format!("in-{id}");
        assert!(
            v2.last_accepted_call_record(id, &input).unwrap().is_some(),
            "{id}: the archived answer stands while the slot is a record"
        );
        let slot = v2.result_path(id);
        let moved = std::fs::read(&slot).unwrap();
        std::fs::remove_file(&slot).unwrap();
        if plant == 0 {
            std::fs::create_dir(&slot).unwrap();
        } else {
            std::fs::write(&outside, &moved).unwrap();
            std::os::unix::fs::symlink(&outside, &slot).unwrap();
        }
        assert!(
            v2.last_accepted_call_record(id, &input).unwrap().is_none(),
            "{id}"
        );
        let reuse = v2.call_record_for_reuse(&agent_call(id), &input).unwrap();
        assert!(reuse.is_none(), "{id}: {reuse:?}");
    }
}
