//! Batch E2: the plan's scope roots are the product area its tasks declare.

use super::*;
use crate::task_universe::WorkflowV2TaskUniverseTask;

fn universe(declared: &[&[&str]]) -> WorkflowV2TaskUniverse {
    WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: Vec::new(),
        tasks: declared
            .iter()
            .enumerate()
            .map(|(at, owns)| WorkflowV2TaskUniverseTask {
                canonical_task_id: format!("TASK-{at}"),
                source_path: format!("tasks/TASK-{at}.md"),
                files_expected_to_change: owns.iter().map(|f| f.to_string()).collect(),
                ..Default::default()
            })
            .collect(),
    }
}

fn repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for path in [
        "Cargo.toml",
        "src/command/trading.rs",
        "src/command/gate.rs",
        "crates/lib-a/Cargo.toml",
        "crates/lib-a/src/deep/store.rs",
        "crates/harness/Cargo.toml",
        "crates/harness/src/lib.rs",
    ] {
        let target = dir.path().join(path);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, "//\n").unwrap();
    }
    dir
}

#[test]
fn an_unpackaged_declared_file_roots_itself_and_its_module_dir_never_its_top_level() {
    let dir = repo();
    let scope = PlanScopeRoots::of(&universe(&[&["src/command/trading.rs"]]), dir.path());
    assert!(scope.covers("src/command/trading.rs"));
    assert!(scope.covers("src/command/trading/split.rs"));
    // The root package is the whole repository: never a scope root.
    assert!(!scope.covers("src/command/gate.rs"));
    assert!(!scope.covers("src/main.rs"));
    assert!(!scope.covers("crates/harness/src/lib.rs"));
}

#[test]
fn a_declared_file_in_a_package_roots_the_package_and_nothing_else() {
    let dir = repo();
    let root = dir.path();
    let absolute = root.join("crates/lib-a/src/deep/store.rs");
    let scope = PlanScopeRoots::of(&universe(&[&[absolute.to_str().unwrap()]]), root);
    assert!(scope.covers("crates/lib-a/src/other.rs"));
    assert!(scope.covers("crates/lib-a/tests/it.rs"));
    assert!(!scope.covers("crates/harness/src/lib.rs"));
    assert!(!scope.covers("src/command/gate.rs"));
    assert_eq!(
        scope.entries().collect::<Vec<_>>(),
        [
            "crates/lib-a",
            "crates/lib-a/src/deep/store",
            "crates/lib-a/src/deep/store.rs"
        ]
    );
}

#[test]
fn nothing_declared_or_nothing_readable_covers_nothing() {
    let dir = repo();
    for declared in [&[][..], &["<placeholder>"][..], &["/elsewhere/x.rs"][..]] {
        let scope = PlanScopeRoots::of(&universe(&[declared]), dir.path());
        assert!(!scope.covers("src/command/gate.rs"), "{declared:?}");
        assert!(
            !scope.covers("crates/lib-a/src/deep/store.rs"),
            "{declared:?}"
        );
    }
}

#[test]
fn a_shared_append_entry_roots_itself_never_its_package() {
    let dir = repo();
    let mut universe = universe(&[&["crates/lib-a/src/deep/store.rs"]]);
    universe.tasks[0].shared_append_target_files = vec!["crates/harness/src/lib.rs".into()];
    let scope = PlanScopeRoots::of(&universe, dir.path());
    assert!(scope.covers("crates/harness/src/lib.rs"));
    assert!(!scope.covers("crates/harness/src/gate.rs"));
    assert!(scope.covers("crates/lib-a/src/other.rs"));
}

#[cfg(unix)]
#[test]
fn a_symbolic_link_inside_a_root_does_not_reach_past_it() {
    let dir = repo();
    let root = dir.path();
    std::os::unix::fs::symlink(root.join("src/command"), root.join("crates/lib-a/src/link"))
        .unwrap();
    let scope = PlanScopeRoots::of(&universe(&[&["crates/lib-a/src/deep/store.rs"]]), root);
    assert!(scope.covers("crates/lib-a/src/link/gate.rs"));
    assert!(!scope.covers_on_disk(root, "crates/lib-a/src/link/gate.rs"));
    assert!(scope.covers_on_disk(root, "crates/lib-a/src/deep/store.rs"));
    assert!(!scope.covers_on_disk(root, "crates/lib-a/src/missing.rs"));
}
