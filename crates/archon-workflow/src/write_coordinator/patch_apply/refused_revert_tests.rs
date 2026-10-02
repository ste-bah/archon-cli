//! Batch L: putting a refused project-input landing back.

use super::super::project_inputs_ledger::run_project_input_landings;
use super::*;
use crate::write_coordinator::project_inputs::write_test_policy;

struct World {
    _temp: tempfile::TempDir,
    project: PathBuf,
    run_root: PathBuf,
}

fn world() -> World {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join("project")).unwrap();
    let project = temp.path().join("project").canonicalize().unwrap();
    let run_root = project.join(".archon/workflows/run");
    std::fs::create_dir_all(&run_root).unwrap();
    std::fs::create_dir_all(project.join("data")).unwrap();
    write_test_policy(&run_root, &project, &["data"]);
    World {
        _temp: temp,
        project,
        run_root,
    }
}

fn hash(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

fn landed(before: &[u8], after: &[u8]) -> ProjectInputLanding {
    ProjectInputLanding {
        stage_id: "fix".into(),
        item_id: "fix-0".into(),
        task_ids: vec!["TASK-A".into()],
        path: "data/registry.json".into(),
        outcome: "applied".into(),
        before: hash(before),
        after: hash(after),
        reason: String::new(),
        at: 1,
        created_dirs: Vec::new(),
    }
}

#[test]
fn a_refused_input_goes_back_to_the_state_it_replaced_and_is_logged() {
    let w = world();
    let registry = w.project.join("data/registry.json");
    crate::write_coordinator::input_tripwire::keep_object(&w.run_root, b"seed\n");
    std::fs::write(&registry, "seed\nfixture\n").unwrap();
    let line = landed(b"seed\n", b"seed\nfixture\n");
    let outcome = revert_input(&w.run_root, &line, "refused by verify-1");
    assert!(
        matches!(outcome, DataRevert::Reverted { .. }),
        "{outcome:?}"
    );
    assert_eq!(std::fs::read_to_string(&registry).unwrap(), "seed\n");
    let log = run_project_input_landings(&w.run_root).unwrap();
    let last = log.last().unwrap();
    assert!(last.reverted(), "{last:?}");
    assert_eq!(last.after, hash(b"seed\n"));
    assert_eq!(last.reason, "refused by verify-1");
}

#[test]
fn a_later_change_that_stands_is_a_conflict_and_nothing_is_written() {
    let w = world();
    let registry = w.project.join("data/registry.json");
    crate::write_coordinator::input_tripwire::keep_object(&w.run_root, b"seed\n");
    std::fs::write(&registry, "seed\nfixture\nlater\n").unwrap();
    let outcome = revert_input(
        &w.run_root,
        &landed(b"seed\n", b"seed\nfixture\n"),
        "refused",
    );
    assert!(
        matches!(&outcome, DataRevert::Conflict(why) if why.contains("a later change stands")),
        "{outcome:?}"
    );
    assert_eq!(
        std::fs::read_to_string(&registry).unwrap(),
        "seed\nfixture\nlater\n"
    );
    assert!(run_project_input_landings(&w.run_root).unwrap().is_empty());
}

#[test]
fn a_created_input_is_removed_and_one_already_put_back_is_left() {
    let w = world();
    let created = w.project.join("data/new.json");
    std::fs::write(&created, "made\n").unwrap();
    let mut line = landed(b"", b"made\n");
    line.path = "data/new.json".into();
    line.before = "absent".into();
    assert!(matches!(
        revert_input(&w.run_root, &line, "refused"),
        DataRevert::Reverted { .. }
    ));
    assert!(!created.exists());
    assert_eq!(
        revert_input(&w.run_root, &line, "refused"),
        DataRevert::Already
    );
}

#[test]
fn a_state_the_run_never_kept_cannot_be_restored() {
    let w = world();
    let registry = w.project.join("data/registry.json");
    std::fs::write(&registry, "seed\nfixture\n").unwrap();
    let outcome = revert_input(
        &w.run_root,
        &landed(b"never kept\n", b"seed\nfixture\n"),
        "refused",
    );
    assert!(
        matches!(&outcome, DataRevert::Conflict(why) if why.contains("was not kept")),
        "{outcome:?}"
    );
    assert_eq!(
        std::fs::read_to_string(&registry).unwrap(),
        "seed\nfixture\n"
    );
}
