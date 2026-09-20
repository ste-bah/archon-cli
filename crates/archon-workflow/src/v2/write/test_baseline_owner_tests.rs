use std::path::Path;

use super::{Ownership, ownership, test_file};
use crate::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};

fn write(root: &Path, rel: &str, body: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

/// A workspace with a root package `app` (`src/`) and one member crate
/// `engine` under `crates/`, laid out the way this module walks it.
fn workspace() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(root, "Cargo.toml", "[workspace]\nmembers = [\"crates/*\"]\n\n[package]\nname = \"app\"\n");
    write(root, "src/main.rs", "fn main() {}\n");
    write(root, "src/command/mod.rs", "");
    write(root, "src/command/run.rs", "");
    write(root, "crates/engine/Cargo.toml", "[package]\nname = \"engine\"\nversion = \"0.1.0\"\n");
    write(root, "crates/engine/src/lib.rs", "");
    write(root, "crates/engine/src/grant.rs", "");
    write(root, "crates/engine/src/grant_tests.rs", "");
    write(root, "crates/engine/src/plan/mod.rs", "");
    write(root, "crates/engine/src/plan/inline.rs", "");
    temp
}

#[test]
fn a_sibling_tests_file_wins_over_the_module_that_declares_it() {
    let ws = workspace();
    assert_eq!(
        test_file(ws.path(), "cargo test -p engine grant", "grant::tests::widens"),
        Some("crates/engine/src/grant_tests.rs".into())
    );
}

#[test]
fn an_inline_tests_module_resolves_to_the_declaring_file() {
    let ws = workspace();
    assert_eq!(
        test_file(ws.path(), "cargo test -p engine", "plan::inline::tests::parses"),
        Some("crates/engine/src/plan/inline.rs".into())
    );
    assert_eq!(
        test_file(ws.path(), "cargo test -p engine", "plan::tests::orders"),
        Some("crates/engine/src/plan/mod.rs".into())
    );
}

#[test]
fn the_root_package_resolves_under_src_and_a_bare_test_reaches_the_crate_root() {
    let ws = workspace();
    assert_eq!(
        test_file(ws.path(), "cargo test --bin app command", "command::run::tests::starts"),
        Some("src/command/run.rs".into())
    );
    assert_eq!(
        test_file(ws.path(), "cargo test -p app", "smoke"),
        Some("src/main.rs".into())
    );
}

#[test]
fn a_command_naming_no_package_resolves_only_when_one_package_holds_the_file() {
    let ws = workspace();
    assert_eq!(
        test_file(ws.path(), "cargo test grant", "grant::tests::widens"),
        Some("crates/engine/src/grant_tests.rs".into())
    );
    // A root-level test exists in BOTH packages: ambiguous.
    assert_eq!(test_file(ws.path(), "cargo test", "tests::smoke"), None);
    // A module path that reaches no file is unresolved, not the crate root.
    assert_eq!(test_file(ws.path(), "cargo test -p engine", "nowhere::at_all"), None);
}

#[test]
fn a_package_the_workspace_does_not_hold_is_unresolvable() {
    let ws = workspace();
    assert_eq!(test_file(ws.path(), "cargo test -p ghost", "grant::tests::widens"), None);
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
            task("TASK-A", &["`crates/engine/src/grant.rs` — the grant", "crates/engine/src/grant_tests.rs"]),
            task("TASK-B", &["crates/engine/src/plan/"]),
            task("TASK-C", &["src/command/run.rs", "crates/engine/src/grant.rs"]),
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
        ownership(Some(&u), &ids(&["TASK-C"]), &[], "crates/engine/src/grant.rs"),
        Ownership::Current
    );
}

#[test]
fn a_file_only_another_task_declares_is_routed_to_it_by_file_or_by_directory() {
    let u = universe();
    assert_eq!(
        ownership(Some(&u), &ids(&["TASK-B"]), &[], "crates/engine/src/grant_tests.rs"),
        Ownership::Other("TASK-A".into())
    );
    assert_eq!(
        ownership(Some(&u), &ids(&["TASK-A"]), &[], "crates/engine/src/plan/inline.rs"),
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
        ownership(None, &ids(&["TASK-A"]), &[], "crates/engine/src/plan/inline.rs"),
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
