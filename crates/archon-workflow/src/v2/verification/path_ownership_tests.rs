//! Issue-85: the host's conclusion about who declares what, computed before
//! dispatch so the universe never travels to a read-only branch.
use serde_json::json;

use super::{
    PATH_OWNERSHIP_INPUT_KEY, declared_paths_of, path_ownership_for,
    stamp_path_ownership_from_universe, stamped,
};
use crate::task_universe::{
    WorkflowV2DeliverableContract, WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask,
};
use crate::v2::WorkflowV2FanoutItem;

fn task(id: &str, expected: &[&str]) -> WorkflowV2TaskUniverseTask {
    WorkflowV2TaskUniverseTask {
        canonical_task_id: id.into(),
        files_expected_to_change: expected
            .iter()
            .map(|path| format!("`{path}` — the deliverable, narrow edits only"))
            .collect(),
        ..Default::default()
    }
}

fn universe(tasks: Vec<WorkflowV2TaskUniverseTask>) -> WorkflowV2TaskUniverse {
    WorkflowV2TaskUniverse {
        schema_version: "1".into(),
        source_roots: Vec::new(),
        tasks,
    }
}

fn item(task_ids: &[&str]) -> WorkflowV2FanoutItem {
    WorkflowV2FanoutItem::read_only(
        "verify-1".to_string(),
        "coder".to_string(),
        crate::v2::WorkflowV2HostCall {
            id: "verification-wave-1-verify-1".into(),
            method: crate::v2::WorkflowV2HostMethod::Agent,
            write_mode: None,
            options: Default::default(),
        },
        json!({"item": {"canonical_task_ids": task_ids}}),
    )
}

/// A prose bullet is read for its path, an appended-to file still counts as
/// declared, and a contract artifact counts too — a file owned only as a
/// contract must never read as nobody's.
#[test]
fn every_way_a_task_can_declare_a_path_counts_as_declared() {
    let mut task = task("TASK-A", &["crates/engine/src/mine.rs"]);
    task.shared_append_target_files = vec!["`crates/engine/src/lib.rs` — one mod line".into()];
    task.deliverable_contracts = vec![WorkflowV2DeliverableContract {
        kind: "report".into(),
        artifact_path: ".archon/reports/summary.json".into(),
        registry_path: Some(".archon/reports/registry.json".into()),
        ..Default::default()
    }];
    let declared = declared_paths_of(&task);
    for path in [
        "crates/engine/src/mine.rs",
        "crates/engine/src/lib.rs",
        ".archon/reports/summary.json",
        ".archon/reports/registry.json",
    ] {
        assert!(declared.contains(path), "{path} missing from {declared:?}");
    }
}

#[test]
fn the_conclusion_splits_this_tasks_paths_from_every_other_tasks() {
    let universe = universe(vec![
        task("TASK-A", &["crates/engine/src/mine.rs"]),
        task("TASK-B", &["docs/plan.md", "crates/engine/src/theirs.rs"]),
    ]);
    let ownership = path_ownership_for(&universe, &["TASK-A".to_string()], None);
    assert_eq!(ownership.own_declared, vec!["crates/engine/src/mine.rs"]);
    let listed: Vec<(String, String)> = ownership
        .declared_elsewhere
        .iter()
        .map(|entry| (entry.path.clone(), entry.owner_task.clone()))
        .collect();
    assert_eq!(
        listed,
        vec![
            ("crates/engine/src/theirs.rs".to_string(), "TASK-B".into()),
            ("docs/plan.md".to_string(), "TASK-B".into()),
        ]
    );
    // Neither list names it, which is how the verifier knows it is nobody's.
    assert!(!ownership.own_declared.iter().any(|p| p == "src/other.rs"));
    assert!(
        !ownership
            .declared_elsewhere
            .iter()
            .any(|entry| entry.path == "src/other.rs")
    );
}

/// A path two tasks declare is the current task's: the answer that refuses
/// the exemption rather than granting it.
#[test]
fn a_path_this_task_also_declares_is_never_listed_as_another_tasks() {
    let shared = "crates/engine/src/shared.rs";
    let universe = universe(vec![task("TASK-A", &[shared]), task("TASK-B", &[shared])]);
    let ownership = path_ownership_for(&universe, &["TASK-A".to_string()], None);
    assert_eq!(ownership.own_declared, vec![shared.to_string()]);
    assert!(ownership.declared_elsewhere.is_empty());
}

#[test]
fn the_stamp_lands_on_the_branch_and_round_trips() {
    let universe = universe(vec![
        task("TASK-A", &["crates/engine/src/mine.rs"]),
        task("TASK-B", &["docs/plan.md"]),
    ]);
    let items = stamp_path_ownership_from_universe(vec![item(&["TASK-A"])], Some(&universe), None);
    let stamp = stamped(&items[0].input).expect("stamped");
    assert_eq!(stamp.own_declared, vec!["crates/engine/src/mine.rs"]);
    assert_eq!(stamp.declared_elsewhere[0].owner_task, "TASK-B");
    // The universe itself never travels: only paths and owning task ids.
    let rendered = items[0].input[PATH_OWNERSHIP_INPUT_KEY].to_string();
    assert!(!rendered.contains("acceptance_criteria"), "{rendered}");
    assert!(!rendered.contains("files_expected_to_change"), "{rendered}");
}

#[test]
fn nothing_is_stamped_without_a_universe_or_without_claimed_tasks() {
    let universe = universe(vec![task("TASK-A", &["crates/engine/src/mine.rs"])]);
    let none = stamp_path_ownership_from_universe(vec![item(&["TASK-A"])], None, None);
    assert!(stamped(&none[0].input).is_none());
    let unclaimed = stamp_path_ownership_from_universe(vec![item(&[])], Some(&universe), None);
    assert!(stamped(&unclaimed[0].input).is_none());
    // A task the universe does not hold declares nothing, so there is
    // nothing to say and no exemption is offered.
    let unknown =
        stamp_path_ownership_from_universe(vec![item(&["TASK-Z"])], Some(&universe), None);
    assert!(stamped(&unknown[0].input).is_some());
}

/// Issue-88: task bodies mix spellings, so the stamp reduces them to one.
/// Before this, a task declaring absolutely rendered absolute entries beside
/// another task's relative ones, and neither list could be compared with the
/// other or with a cited path.
#[test]
fn declared_paths_are_rendered_repository_relative_whatever_the_task_wrote() {
    let root = std::path::Path::new("/repo");
    let mut absolute = task("TASK-A", &[]);
    absolute.files_expected_to_change =
        vec!["`/repo/crates/engine/tests/surface.rs` — exists (492 lines)".into()];
    let mut relative = task("TASK-B", &["docs/plan.md"]);
    relative.deliverable_contracts = vec![WorkflowV2DeliverableContract {
        kind: "json-artifact".into(),
        artifact_path: ".archon/data/latest.json".into(),
        ..Default::default()
    }];
    let universe = universe(vec![absolute, relative]);
    let ownership = path_ownership_for(&universe, &["TASK-A".to_string()], Some(root));
    assert_eq!(
        ownership.own_declared,
        vec!["crates/engine/tests/surface.rs"]
    );
    let elsewhere: Vec<String> = ownership
        .declared_elsewhere
        .iter()
        .map(|entry| entry.path.clone())
        .collect();
    assert_eq!(elsewhere, vec![".archon/data/latest.json", "docs/plan.md"]);
    assert!(
        !elsewhere.iter().any(|path| path.starts_with('/')),
        "{elsewhere:?}"
    );
}

#[test]
fn a_declared_entry_that_is_not_a_path_refuses_the_whole_lookup() {
    use super::{DeclaredPathForm, canonical_declared_paths, declared_path_form};
    let root = std::path::Path::new("/repo");
    assert_eq!(
        declared_path_form("/repo/crates/a.rs", root),
        DeclaredPathForm::Repo("crates/a.rs".into())
    );
    assert_eq!(
        declared_path_form("crates/a.rs", root),
        DeclaredPathForm::Repo("crates/a.rs".into())
    );
    assert_eq!(
        declared_path_form("/elsewhere/project/data.json", root),
        DeclaredPathForm::Outside
    );
    for unusable in ["", "   ", "../escape.rs", "crates/<dataset-id>/a.rs"] {
        assert_eq!(
            declared_path_form(unusable, root),
            DeclaredPathForm::Unusable,
            "{unusable}"
        );
    }
    // One unreadable entry anywhere refuses the whole map: it might be the
    // path a caller is asking about.
    let mut templated = task("TASK-A", &[]);
    templated.files_expected_to_change = vec!["`crates/<dataset-id>/a.rs`".into()];
    assert!(canonical_declared_paths(&universe(vec![templated]), root).is_none());
    assert!(
        canonical_declared_paths(&universe(vec![task("TASK-A", &["crates/a.rs"])]), root).is_some()
    );
}
