//! Issue-292 follow-ups: a refused store entry is a visible gap, never a
//! fatal error and never a warning on every read.
//!
//! - L2: a link loop (`ELOOP`) in a call's directory or its `superseded/`
//!   archive is refused and reported; restart never fails on it.
//! - L1: an agent's notes kept directly in `branches/` are never call
//!   directories; fan-outs skip them silently. Only a candidate entry that
//!   is refused is reported.
//! - L3: each refused entry is reported once per path in a process, not on
//!   every read.
#![cfg(unix)]

#[path = "support/branch_revocation.rs"]
mod branch_revocation;
#[path = "support/restart_run.rs"]
mod restart_run;
#[path = "support/store_entries.rs"]
mod store_entries;

use std::os::unix::fs::symlink;
use std::path::PathBuf;

use archon_workflow::v2::restart::restart_generated_v2_task;
use archon_workflow::v2::store_file::{
    StoreRefusal, classify_store_entry, read_store_file_in, store_file_refusal,
};
use archon_workflow::{WorkflowStore, WorkflowV2ResultStore, WorkflowV2Status};
use branch_revocation::*;
use restart_run::{accepted, generated_run, v2_store};
use store_entries::{bounded, mkfifo, write_oversized};

/// A run whose `T-A` landed and was superseded, with its call directory.
struct Archived {
    temp: tempfile::TempDir,
    store: WorkflowStore,
    run: archon_workflow::WorkflowRun,
    v2: WorkflowV2ResultStore,
    call_dir: PathBuf,
}

fn archived_run() -> Archived {
    let temp = tempfile::tempdir().unwrap();
    let (store, run) = generated_run(&temp, &[CALL]);
    let v2 = v2_store(&store, &run);
    landed_then_superseded(&v2, "T-A");
    let current = v2.branch_outcome_path(CALL, &item("T-A").id);
    let call_dir = current.parent().unwrap().to_path_buf();
    Archived {
        temp,
        store,
        run,
        v2,
        call_dir,
    }
}

/// One fan-out's branch readers: the superseded outcomes and a split.
fn fan_out(v2: &WorkflowV2ResultStore) -> (usize, Vec<String>) {
    let superseded = v2.load_superseded_branch_outcomes().len();
    let (reused, _) = split(v2, &["T-A", "T-B"]);
    (superseded, reused)
}

fn landed_b() -> Vec<u8> {
    serde_json::to_vec(&outcome("T-B", WorkflowV2Status::Accepted, "H1", true)).unwrap()
}

// ---- L2: a link loop is a refusal, never fatal ----

#[test]
fn a_self_link_in_the_archive_never_fails_restart() {
    let run = archived_run();
    let link = run.call_dir.join("superseded/loop.json");
    symlink("loop.json", &link).unwrap();
    let (store, workflow) = (run.store.clone(), run.run.clone());
    let (restarts, warnings) = bounded(&[], move || {
        let run = workflow;
        let first = restart_generated_v2_task(&store, &run, "T-A").map(|_| ());
        let later = restart_generated_v2_task(&store, &run, "T-A").map(|_| ());
        (
            first.map_err(|e| e.to_string()),
            later.map_err(|e| e.to_string()),
        )
    });
    restarts.0.expect("the first restart failed on a link loop");
    restarts.1.expect("a later restart failed on a link loop");
    assert!(warnings.name(&link), "{:?}", warnings.lines());
    assert_revoked(&run.v2, "T-A");
    assert!(std::fs::symlink_metadata(&link).is_ok(), "never planned");
}

#[test]
fn a_link_cycle_among_current_outcomes_never_fails_restart() {
    let run = archived_run();
    let (a, b) = (
        run.call_dir.join("cyc-a.json"),
        run.call_dir.join("cyc-b.json"),
    );
    symlink("cyc-b.json", &a).unwrap();
    symlink("cyc-a.json", &b).unwrap();
    let (store, workflow) = (run.store.clone(), run.run.clone());
    let (restarted, warnings) = bounded(&[], move || {
        let run = workflow;
        restart_generated_v2_task(&store, &run, "T-A").map_err(|e| e.to_string())
    });
    restarted.expect("restart failed on a link cycle");
    assert!(
        warnings.name(&a) && warnings.name(&b),
        "{:?}",
        warnings.lines()
    );
    assert_revoked(&run.v2, "T-A");
}

#[test]
fn a_link_loop_is_refused_and_never_counted() {
    let run = archived_run();
    let link = run.call_dir.join("superseded/loop.json");
    symlink("loop.json", &link).unwrap();
    // The fan-out runs first: the path is reported once in this process.
    let v2 = run.v2.clone();
    let ((superseded, reused), warnings) = bounded(&[], move || fan_out(&v2));
    assert_eq!(superseded, 1, "only T-A's archived record counts");
    assert_eq!(reused, vec![item("T-A").id]);
    assert!(warnings.name(&link), "{:?}", warnings.lines());
    assert_eq!(
        classify_store_entry(&link, &run.call_dir).unwrap(),
        Some(StoreRefusal::LinkLoop)
    );
    let error = read_store_file_in(&link, &run.call_dir).expect_err("a loop is no record");
    assert_eq!(store_file_refusal(&error), Some(StoreRefusal::LinkLoop));
}

// ---- L1: notes kept in branches/ are skipped silently ----

/// Plant `names` as regular files directly in `branches/` and run three
/// fan-outs: none names them, and the archive still reads.
fn assert_notes_silent(names: &[&str]) {
    let run = archived_run();
    let branches = run.call_dir.parent().unwrap().to_path_buf();
    let notes: Vec<PathBuf> = names.iter().map(|name| branches.join(name)).collect();
    for note in &notes {
        std::fs::write(note, b"agent notes, not a record").unwrap();
    }
    let v2 = run.v2.clone();
    let (counted, warnings) = bounded(&[], move || {
        (0..3).map(|_| fan_out(&v2)).collect::<Vec<_>>()
    });
    // The first fan-out sees only T-A's archived record (a later one also
    // sees what the first archived).
    assert_eq!(counted[0], (1, vec![item("T-A").id]), "{counted:?}");
    assert!(
        counted
            .iter()
            .all(|(_, reused)| reused == &[item("T-A").id])
    );
    for note in &notes {
        assert!(!warnings.name(note), "{note:?}: {:?}", warnings.lines());
    }
    drop(run.temp);
}

#[test]
fn markdown_notes_in_branches_never_warn_on_a_fan_out() {
    assert_notes_silent(&["agents-4-0-fmt-revert-notes.md"]);
}

#[test]
fn json_named_and_bare_files_in_branches_never_warn_on_a_fan_out() {
    assert_notes_silent(&["summary.json", "NOTES", "handoff.txt"]);
}

#[test]
fn a_refused_candidate_still_warns_next_to_silent_notes() {
    let run = archived_run();
    let note = run.call_dir.parent().unwrap().join("notes.md");
    std::fs::write(&note, b"notes").unwrap();
    let fifo = run.call_dir.join("superseded/planted.json");
    mkfifo(&fifo);
    let v2 = run.v2.clone();
    let ((superseded, _), warnings) = bounded(&[fifo.clone()], move || fan_out(&v2));
    assert_eq!(superseded, 1);
    assert!(warnings.name(&fifo), "{:?}", warnings.lines());
    assert!(!warnings.name(&note), "{:?}", warnings.lines());
}

// ---- L3: one warning per refused path per process ----

#[test]
fn a_refused_archive_entry_warns_once_over_many_fan_outs() {
    let run = archived_run();
    let fifo = run.call_dir.join("superseded/planted.json");
    mkfifo(&fifo);
    let v2 = run.v2.clone();
    let (_, warnings) = bounded(&[fifo.clone()], move || (0..4).map(|_| fan_out(&v2)).last());
    assert_eq!(warnings.count(&fifo), 1, "{:?}", warnings.lines());
}

#[test]
fn each_refused_path_warns_once_and_none_is_lost() {
    let run = archived_run();
    let archive = run.call_dir.join("superseded");
    let big = archive.join("big.json");
    write_oversized(&big, &landed_b());
    let outside = run.temp.path().join("landing-b.json");
    std::fs::write(&outside, landed_b()).unwrap();
    let link = archive.join("outside.json");
    symlink(&outside, &link).unwrap();
    let v2 = run.v2.clone();
    let (_, warnings) = bounded(&[], move || (0..3).map(|_| fan_out(&v2)).last());
    assert_eq!(warnings.count(&big), 1, "{:?}", warnings.lines());
    assert_eq!(warnings.count(&link), 1, "{:?}", warnings.lines());
}

#[test]
fn a_refused_history_entry_warns_once_over_many_lookups() {
    let temp = tempfile::tempdir().unwrap();
    let (store, run) = generated_run(&temp, &[CALL]);
    let v2 = v2_store(&store, &run);
    v2.save_call_record(&accepted("call-x", "T-X")).unwrap();
    let history = v2.call_history_dir("call-x");
    std::fs::create_dir_all(&history).unwrap();
    let fifo = history.join("planted.json");
    mkfifo(&fifo);
    let reader = v2.clone();
    let (attempts, warnings) = bounded(&[fifo.clone()], move || {
        (0..3)
            .map(|_| reader.next_attempt("call-x").map_err(|e| e.to_string()))
            .collect::<Result<Vec<_>, _>>()
    });
    attempts.unwrap();
    assert_eq!(warnings.count(&fifo), 1, "{:?}", warnings.lines());
}
