//! Child module of `artifact_path_guard`: the directory rule for declared
//! artifacts, before and after issue-68.
//!
//! Two live failures shaped this. TASK-TDL-080 declared
//! `.archon/trading-lab/data/coverage/history/` (trailing slash), produced it
//! holding an archived snapshot, and was failed three times for "is a
//! directory, not the declared file". TASK-TRADING-010 declared a contract
//! `artifact_path: .archon/trading-lab/data/runs` (no trailing slash), wrote
//! `runs/spec-<id>/` holding four files, declared that directory, and was
//! failed twice with the same words. Both were correct work no retry could
//! pass.

use super::{artifact_file_defect, declared_artifact_defect};

fn run_directory_with_output(root: &std::path::Path) -> std::path::PathBuf {
    let run = root.join("runs").join("spec-1");
    std::fs::create_dir_all(&run).expect("mkdir");
    std::fs::write(run.join("report.json"), "{\"trades\":3}\n").expect("report");
    run
}

/// TASK-TDL-080: a declared directory (trailing slash) holding evidence.
#[test]
fn a_declared_directory_holding_evidence_passes() {
    let dir = tempfile::tempdir().expect("root");
    let history = dir.path().join("history");
    std::fs::create_dir_all(&history).expect("mkdir");
    std::fs::write(history.join("20260814T111500Z.json"), "{}\n").expect("archive");
    assert_eq!(
        declared_artifact_defect("coverage/history/", &history, false),
        None
    );
}

/// Issue-68: the same directory is evidence when the contract path carried no
/// trailing slash and no directory flag — the coder's `runs/spec-1` under a
/// contract `runs`. Non-emptiness is what makes it evidence, not the spelling
/// of the contract.
#[test]
fn a_non_empty_directory_is_evidence_without_a_trailing_slash_or_flag() {
    let dir = tempfile::tempdir().expect("root");
    let run = run_directory_with_output(dir.path());
    assert_eq!(artifact_file_defect(&run), None);
    assert_eq!(declared_artifact_defect("runs/spec-1", &run, false), None);
    assert_eq!(declared_artifact_defect("runs/spec-1", &run, true), None);
    assert_eq!(
        declared_artifact_defect("runs", &dir.path().join("runs"), false),
        None,
        "the contract path itself, a directory of run directories, is evidence"
    );
}

/// The same declaration with nothing on disk is "does not exist", not any
/// directory wording.
#[test]
fn a_declared_run_directory_that_was_never_written_does_not_exist() {
    let dir = tempfile::tempdir().expect("root");
    let absent = dir.path().join("runs").join("spec-1");
    assert_eq!(
        declared_artifact_defect("runs/spec-1", &absent, false),
        Some("does not exist")
    );
    assert_eq!(artifact_file_defect(&absent), Some("does not exist"));
}

/// The litter this module exists to stop is still refused: an EMPTY directory
/// — `mkdir -p` and nothing written — is not evidence, with or without the
/// directory flag.
#[test]
fn an_empty_directory_is_still_refused() {
    let dir = tempfile::tempdir().expect("root");
    let empty = dir.path().join("history");
    std::fs::create_dir_all(&empty).expect("mkdir");
    assert_eq!(
        declared_artifact_defect("coverage/history/", &empty, false),
        Some("is an empty directory")
    );
    assert_eq!(
        declared_artifact_defect("coverage/history", &empty, false),
        Some("is an empty directory")
    );
    assert_eq!(artifact_file_defect(&empty), Some("is an empty directory"));
}

/// Issue #168's exact shape: a nested tree of directories with nothing in it.
/// The outer directory HAS an entry, and still evidences nothing. A tree whose
/// only files are zero bytes is the same defect by another road.
#[test]
fn a_tree_holding_no_non_empty_file_is_not_evidence() {
    let dir = tempfile::tempdir().expect("root");
    let outer = dir.path().join("A gap-audit report");
    std::fs::create_dir_all(outer.join("environment").join("readiness blockers")).expect("mkdir");
    assert_eq!(
        artifact_file_defect(&outer),
        Some("is a directory holding no non-empty file")
    );
    std::fs::write(outer.join("environment").join("trades.jsonl"), "").expect("empty file");
    assert_eq!(
        declared_artifact_defect("runs/spec-1", &outer, false),
        Some("is a directory holding no non-empty file")
    );
    std::fs::write(outer.join("environment").join("report.json"), "{}\n").expect("report");
    assert_eq!(
        artifact_file_defect(&outer),
        None,
        "one non-empty file anywhere under it is evidence"
    );
}

/// A file where the contract explicitly asked for a directory is still wrong.
#[test]
fn a_file_where_a_directory_was_declared_is_refused() {
    let dir = tempfile::tempdir().expect("root");
    let f = dir.path().join("history");
    std::fs::write(&f, "not a dir\n").expect("write");
    assert_eq!(
        declared_artifact_defect("coverage/history/", &f, false),
        Some("is not a directory, but the contract declares one")
    );
    assert_eq!(
        declared_artifact_defect("coverage/history", &f, true),
        Some("is not a directory, but the contract declares one")
    );
}

/// Ordinary file declarations behave exactly as before: bytes pass, none fail.
#[test]
fn a_declared_file_is_unchanged() {
    let dir = tempfile::tempdir().expect("root");
    let f = dir.path().join("latest.json");
    std::fs::write(&f, "{}\n").expect("write");
    assert_eq!(
        declared_artifact_defect("coverage/latest.json", &f, false),
        None
    );
    let empty = dir.path().join("empty.json");
    std::fs::write(&empty, "").expect("write");
    assert_eq!(
        declared_artifact_defect("coverage/empty.json", &empty, false),
        Some("is an empty file")
    );
    assert_eq!(artifact_file_defect(&empty), Some("is an empty file"));
}

/// The live shape: by the time the check runs the separator is gone, so the
/// intent arrives as its own value. The flag still matters for what it
/// REFUSES (a file), not for what it accepts — a non-empty directory passes
/// either way since issue-68.
#[test]
fn the_intent_flag_works_when_the_separator_is_already_lost() {
    let dir = tempfile::tempdir().expect("root");
    let history = dir.path().join("history");
    std::fs::create_dir_all(&history).expect("mkdir");
    std::fs::write(history.join("20260814T111500Z.json"), "{}\n").expect("archive");

    // Absolute, no trailing slash — what the completion check receives.
    let declared = history.display().to_string();
    assert_eq!(declared_artifact_defect(&declared, &history, false), None);
    assert_eq!(declared_artifact_defect(&declared, &history, true), None);
}
