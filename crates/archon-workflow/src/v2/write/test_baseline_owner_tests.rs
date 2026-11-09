use std::path::Path;

use super::{Ownership, ownership};
fn test_file(root: &Path, command: &str, id: &str) -> Option<String> {
    let commit = super::super::test_baseline_run_base::tests::head(root);
    super::super::test_baseline_owner_at::test_file_at(root, Some(&commit), command, id)
}
use crate::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};

fn write(root: &Path, rel: &str, body: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

/// A workspace with a root package `app` (`src/`) and one member crate
/// `engine` under `crates/`, used by the explicit-path cases.
fn workspace() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(
        root,
        "Cargo.toml",
        "[workspace]\nmembers = [\"crates/*\"]\n\n[package]\nname = \"app\"\n",
    );
    write(root, "src/main.rs", "fn main() {}\n");
    write(root, "src/command/mod.rs", "");
    write(root, "src/command/run.rs", "");
    write(
        root,
        "crates/engine/Cargo.toml",
        "[package]\nname = \"engine\"\nversion = \"0.1.0\"\n",
    );
    write(root, "crates/engine/src/lib.rs", "");
    write(root, "crates/engine/src/grant.rs", "");
    write(root, "crates/engine/src/grant_tests.rs", "");
    write(root, "crates/engine/src/plan/mod.rs", "");
    write(root, "crates/engine/src/plan/inline.rs", "");
    write(root, "crates/engine/tests/gates.rs", "");
    write(root, "crates/engine/tests/wide/main.rs", "");
    write(root, "crates/engine/tests/wide/cases.rs", "");
    for args in [
        &["init", "-q"][..],
        &["config", "user.name", "t"],
        &["config", "user.email", "t@example.invalid"],
        &["add", "."],
        &["commit", "-qm", "fixture"],
    ] {
        crate::write_coordinator::worktree_isolation::run_git(args, root).unwrap();
    }
    temp
}

#[test]
fn a_sibling_tests_file_needs_an_explicit_path() {
    let ws = workspace();
    assert_eq!(
        test_file(
            ws.path(),
            "cargo test -p engine grant",
            "crates/engine/src/grant_tests.rs::case"
        ),
        Some("crates/engine/src/grant_tests.rs".into())
    );
    assert_eq!(
        test_file(
            ws.path(),
            "cargo test -p engine grant",
            "grant::tests::widens"
        ),
        None
    );
}

#[test]
fn an_inline_tests_module_needs_an_explicit_path() {
    let ws = workspace();
    assert_eq!(
        test_file(
            ws.path(),
            "cargo test -p engine",
            "crates/engine/src/plan/inline.rs::case"
        ),
        Some("crates/engine/src/plan/inline.rs".into())
    );
    assert_eq!(
        test_file(
            ws.path(),
            "cargo test -p engine",
            "plan::inline::tests::parses"
        ),
        None
    );
    assert_eq!(
        test_file(
            ws.path(),
            "cargo test -p engine",
            "crates/engine/src/plan/mod.rs::case"
        ),
        Some("crates/engine/src/plan/mod.rs".into())
    );
    assert_eq!(
        test_file(ws.path(), "cargo test -p engine", "plan::tests::orders"),
        None
    );
}

#[test]
fn an_explicit_root_file_resolves_and_bare_modules_do_not() {
    let ws = workspace();
    assert_eq!(
        test_file(
            ws.path(),
            "cargo test --bin app command",
            "src/command/run.rs::case"
        ),
        Some("src/command/run.rs".into())
    );
    assert_eq!(
        test_file(
            ws.path(),
            "cargo test --bin app command",
            "command::run::tests::starts"
        ),
        None
    );
    // An inline `tests` module at the crate root is the root file's.
    assert_eq!(
        test_file(ws.path(), "cargo test -p app", "src/main.rs::case"),
        Some("src/main.rs".into())
    );
    assert_eq!(
        test_file(ws.path(), "cargo test -p app", "tests::smoke"),
        None
    );
    // A bare id carries no module path — what an integration binary
    // reports — and never claims the crate root (Issue-73).
    assert_eq!(test_file(ws.path(), "cargo test -p app", "smoke"), None);
}

#[test]
fn an_explicit_integration_path_resolves_without_inventing_a_root() {
    let ws = workspace();
    for command in [
        "cargo nextest run -p engine --test gates",
        "cargo test -p engine --test=gates",
    ] {
        assert_eq!(
            test_file(
                ws.path(),
                command,
                "crates/engine/tests/gates.rs::current_artifact_integrity_is_required"
            ),
            Some("crates/engine/tests/gates.rs".into()),
            "{command}"
        );
        assert_eq!(
            test_file(ws.path(), command, "current_artifact_integrity_is_required"),
            None,
            "{command}"
        );
    }
    // Explicit paths name files; target selectors do not supply a missing path.
    assert_eq!(
        test_file(
            ws.path(),
            "cargo test -p engine --test wide",
            "crates/engine/tests/wide/cases.rs::case"
        ),
        Some("crates/engine/tests/wide/cases.rs".into())
    );
    assert_eq!(
        test_file(
            ws.path(),
            "cargo test -p engine --test wide",
            "cases::tests::widens"
        ),
        None
    );
    assert_eq!(
        test_file(
            ws.path(),
            "cargo test -p engine --test wide",
            "crates/engine/tests/wide/main.rs::case"
        ),
        Some("crates/engine/tests/wide/main.rs".into())
    );
    assert_eq!(
        test_file(ws.path(), "cargo test -p engine --test wide", "smoke"),
        None
    );
    // Unlocated IDs never invent a target or package root.
    assert_eq!(
        test_file(ws.path(), "cargo test -p engine --test ghost", "smoke"),
        None
    );
    assert_eq!(
        test_file(
            ws.path(),
            "cargo test -p engine --test ghost",
            "grant::tests::widens"
        ),
        None
    );
}

#[test]
fn an_explicit_path_does_not_require_a_package_selection() {
    let ws = workspace();
    assert_eq!(
        test_file(
            ws.path(),
            "cargo test grant",
            "crates/engine/src/grant_tests.rs::case"
        ),
        Some("crates/engine/src/grant_tests.rs".into())
    );
    assert_eq!(
        test_file(ws.path(), "cargo test grant", "grant::tests::widens"),
        None
    );
    // A bare module ID supplies no explicit file path.
    assert_eq!(test_file(ws.path(), "cargo test", "tests::smoke"), None);
    // A module path that reaches no file is unresolved, not the crate root.
    assert_eq!(
        test_file(ws.path(), "cargo test -p engine", "nowhere::at_all"),
        None
    );
}

#[test]
fn a_command_cannot_resolve_an_unlocated_module_id() {
    let ws = workspace();
    assert_eq!(
        test_file(ws.path(), "cargo test -p ghost", "grant::tests::widens"),
        None
    );
}

fn universe() -> WorkflowV2TaskUniverse {
    let task = |id: &str, files: &[&str]| WorkflowV2TaskUniverseTask {
        canonical_task_id: id.into(),
        source_path: format!("tasks/{id}.md"),
        files_expected_to_change: files.iter().map(|f| f.to_string()).collect(),
        ..Default::default()
    };
    WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: Vec::new(),
        tasks: vec![
            task(
                "TASK-A",
                &[
                    "`crates/engine/src/grant.rs` — the grant",
                    "crates/engine/src/grant_tests.rs",
                ],
            ),
            task("TASK-B", &["crates/engine/src/plan/"]),
            task(
                "TASK-C",
                &["src/command/run.rs", "crates/engine/src/grant.rs"],
            ),
        ],
    }
}

fn ids(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

#[test]
fn a_file_the_current_task_declares_is_its_own_even_when_another_task_declares_it_too() {
    let u = universe();
    assert_eq!(
        ownership(
            Some(&u),
            &ids(&["TASK-C"]),
            &[],
            "crates/engine/src/grant.rs"
        ),
        Ownership::Current
    );
}

#[test]
fn a_file_only_another_task_declares_is_routed_to_it_by_file_or_by_directory() {
    let u = universe();
    assert_eq!(
        ownership(
            Some(&u),
            &ids(&["TASK-B"]),
            &[],
            "crates/engine/src/grant_tests.rs"
        ),
        Ownership::Other("TASK-A".into())
    );
    assert_eq!(
        ownership(
            Some(&u),
            &ids(&["TASK-A"]),
            &[],
            "crates/engine/src/plan/inline.rs"
        ),
        Ownership::Other("TASK-B".into())
    );
}

#[test]
fn a_file_nobody_declares_and_a_missing_universe_are_both_unowned() {
    let u = universe();
    assert_eq!(
        ownership(Some(&u), &ids(&["TASK-A"]), &[], "crates/engine/src/lib.rs"),
        Ownership::Unowned
    );
    assert_eq!(
        ownership(
            None,
            &ids(&["TASK-A"]),
            &[],
            "crates/engine/src/plan/inline.rs"
        ),
        Ownership::Unowned
    );
}

#[test]
fn the_branch_declared_targets_count_as_its_own_before_the_universe_is_asked() {
    let u = universe();
    assert_eq!(
        ownership(
            Some(&u),
            &ids(&["TASK-A"]),
            &["crates/engine/src/plan/inline.rs".to_string()],
            "crates/engine/src/plan/inline.rs"
        ),
        Ownership::Current
    );
}
