use super::*;
use crate::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};

const REPO: &str = "/repo";

fn task(task_id: &str, declared: &[&str]) -> WorkflowV2TaskUniverseTask {
    WorkflowV2TaskUniverseTask {
        canonical_task_id: task_id.to_string(),
        files_expected_to_change: declared.iter().map(|entry| (*entry).to_string()).collect(),
        ..Default::default()
    }
}

fn universe(tasks: Vec<WorkflowV2TaskUniverseTask>) -> WorkflowV2TaskUniverse {
    WorkflowV2TaskUniverse {
        schema_version: "workflow-v2-task-universe-v1".to_string(),
        source_roots: Vec::new(),
        tasks,
    }
}

fn item(tasks: &[&str], targets: &[&str]) -> Value {
    serde_json::json!({ "canonical_task_ids": tasks, "target_files": targets })
}

fn roots() -> Vec<String> {
    vec![".artifacts".to_string()]
}

fn added(universe: &WorkflowV2TaskUniverse, item: &Value) -> Vec<String> {
    task_declared_code_targets_for_item(universe, item, &roots(), Some(Path::new(REPO)))
}

/// The deadlock: a branch remediating a task is dispatched with one of the
/// task's declared files. The two it was never given are the two the verifier
/// keeps naming, so the task can neither pass nor fail. All of them are now
/// granted.
#[test]
fn a_declared_file_no_attempt_ever_touched_is_granted() {
    let universe = universe(vec![task(
        "TASK-A-001",
        &[
            "crates/pkg/src/owner.rs",
            "crates/pkg/src/helper/methods.rs",
            "crates/pkg/src/helper/helper_tests/unit.rs",
        ],
    )]);
    let item = item(&["TASK-A-001"], &["crates/pkg/src/owner.rs"]);
    assert_eq!(
        added(&universe, &item),
        vec![
            "crates/pkg/src/helper/methods.rs".to_string(),
            "crates/pkg/src/helper/helper_tests/unit.rs".to_string(),
        ]
    );
}

/// A path no task declaration mentions — a module an earlier attempt split
/// out — is not something this reports on: it is the item's already and stays
/// there. Only the missing declared file is added. (The union itself is
/// asserted where it is performed, in `write::universe_stamps`.)
#[test]
fn a_path_outside_the_declaration_is_left_alone() {
    let universe = universe(vec![task(
        "TASK-A-001",
        &[
            "crates/pkg/src/owner.rs",
            "crates/pkg/src/helper/methods.rs",
        ],
    )]);
    let item = item(
        &["TASK-A-001"],
        &["crates/pkg/src/owner.rs", "crates/pkg/src/owner/split.rs"],
    );
    assert_eq!(
        added(&universe, &item),
        vec!["crates/pkg/src/helper/methods.rs".to_string()]
    );
}

/// No scope widening: a file another task declares stays that task's, so the
/// gate refuses it exactly as before.
#[test]
fn another_tasks_declared_file_is_never_granted() {
    let universe = universe(vec![
        task("TASK-A-001", &["crates/pkg/src/owner.rs"]),
        task("TASK-A-002", &["crates/pkg/src/other.rs"]),
    ]);
    let item = item(&["TASK-A-001"], &["crates/pkg/src/owner.rs"]);
    assert!(added(&universe, &item).is_empty());
}

/// Task files write these entries absolute as freely as relative, and the
/// grant is relative. An absolute declaration under the repository root
/// becomes its relative form; one outside the repository is not a target.
#[test]
fn an_absolute_declaration_normalises_into_the_relative_grant() {
    let universe = universe(vec![task(
        "TASK-A-001",
        &[
            "/repo/crates/pkg/src/helper/methods.rs",
            "/elsewhere/crates/pkg/src/outside.rs",
        ],
    )]);
    let item = item(&["TASK-A-001"], &["crates/pkg/src/owner.rs"]);
    assert_eq!(
        added(&universe, &item),
        vec!["crates/pkg/src/helper/methods.rs".to_string()]
    );
}

/// The entries are prose as often as paths: a backticked path followed by a
/// note about it. The path is taken, the note is not.
#[test]
fn a_prose_entry_yields_its_path() {
    let universe = universe(vec![task(
        "TASK-A-001",
        &["`/repo/crates/pkg/src/helper/methods.rs` — exists (357 lines)"],
    )]);
    let item = item(&["TASK-A-001"], &["crates/pkg/src/owner.rs"]);
    assert_eq!(
        added(&universe, &item),
        vec!["crates/pkg/src/helper/methods.rs".to_string()]
    );
}

/// Nothing is invented from an entry that names no single repository path.
#[test]
fn an_unreadable_entry_grants_nothing() {
    let universe = universe(vec![task(
        "TASK-A-001",
        &[
            "<the module that owns the writer>",
            "crates/pkg/src/**/*.rs",
            "../outside/escape.rs",
            "crates/pkg/src/dir/",
            "",
        ],
    )]);
    let item = item(&["TASK-A-001"], &["crates/pkg/src/owner.rs"]);
    assert!(added(&universe, &item).is_empty());
}

/// A declared path under a project artifact root is a produced artifact,
/// admitted by the artifact path, and never a repository write target.
#[test]
fn a_project_artifact_is_not_admitted_as_code() {
    let universe = universe(vec![task(
        "TASK-A-001",
        &["/repo/.artifacts/lab/report.json"],
    )]);
    let item = item(&["TASK-A-001"], &["crates/pkg/src/owner.rs"]);
    assert!(added(&universe, &item).is_empty());
}

/// An item that owns no repository code is artifact-only and must not acquire
/// code writes here.
#[test]
fn an_artifact_only_item_gains_nothing() {
    let universe = universe(vec![task("TASK-A-001", &["crates/pkg/src/owner.rs"])]);
    let item = item(&["TASK-A-001"], &[]);
    assert!(added(&universe, &item).is_empty());
}

/// An item claiming no task has no declaration to stand on.
#[test]
fn an_item_with_no_task_gains_nothing() {
    let universe = universe(vec![task("TASK-A-001", &["crates/pkg/src/helper.rs"])]);
    let item = item(&[], &["crates/pkg/src/owner.rs"]);
    assert!(added(&universe, &item).is_empty());
}

/// A path the item already lists under the other spelling is not added twice.
#[test]
fn an_existing_target_is_not_repeated() {
    let universe = universe(vec![task(
        "TASK-A-001",
        &["/repo/crates/pkg/src/owner.rs", "crates/pkg/src/helper.rs"],
    )]);
    let item = item(
        &["TASK-A-001"],
        &["/repo/crates/pkg/src/owner.rs", "crates/pkg/src/helper.rs"],
    );
    assert!(added(&universe, &item).is_empty());
}

/// The whole floor, for the caller that replaces an item's targets: once the
/// floor is stamped, the difference against the item is empty, and rebuilding
/// the scope from the difference would drop it again.
#[test]
fn the_full_floor_survives_a_scope_replacement() {
    let universe = universe(vec![task(
        "TASK-A-001",
        &["crates/pkg/src/owner.rs", "crates/pkg/src/helper.rs"],
    )]);
    let item = item(
        &["TASK-A-001"],
        &["crates/pkg/src/owner.rs", "crates/pkg/src/helper.rs"],
    );
    assert!(added(&universe, &item).is_empty());
    assert_eq!(
        task_declared_repository_paths(&universe, &item, &roots(), Some(Path::new(REPO))),
        vec![
            "crates/pkg/src/owner.rs".to_string(),
            "crates/pkg/src/helper.rs".to_string(),
        ]
    );
}

/// With no repository root an absolute entry cannot be resolved; a relative
/// one is already in the grant's language.
#[test]
fn without_a_root_only_relative_entries_resolve() {
    let universe = universe(vec![task(
        "TASK-A-001",
        &[
            "/repo/crates/pkg/src/absolute.rs",
            "crates/pkg/src/helper.rs",
        ],
    )]);
    let item = item(&["TASK-A-001"], &["crates/pkg/src/owner.rs"]);
    assert_eq!(
        task_declared_code_targets_for_item(&universe, &item, &roots(), None),
        vec!["crates/pkg/src/helper.rs".to_string()]
    );
}

/// A declared entry that is a directory on disk is not admitted: targets are
/// matched by containment, so admitting one would grant everything beneath
/// it. A path that does not exist yet is a file the task has still to create
/// and is admitted.
#[test]
fn an_existing_directory_is_not_admitted_as_a_file() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    std::fs::create_dir_all(root.join("crates/pkg/src")).expect("tree");
    let universe = universe(vec![task(
        "TASK-A-001",
        &["crates/pkg/src", "crates/pkg/src/created_later.rs"],
    )]);
    let item = item(&["TASK-A-001"], &["crates/pkg/src/owner.rs"]);
    assert_eq!(
        task_declared_code_targets_for_item(&universe, &item, &roots(), Some(root)),
        vec!["crates/pkg/src/created_later.rs".to_string()]
    );
}
