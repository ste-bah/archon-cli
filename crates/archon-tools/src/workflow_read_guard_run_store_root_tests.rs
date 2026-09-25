//! A call whose working root is the PROJECT root — serial writes and scope
//! discovery run there — has the whole run store beneath its workspace. The
//! workspace exemption must not swallow the store, or the rule is inert for
//! exactly those calls.
use super::RunStoreScope;
use crate::workflow_read_guard::{WorkflowReadGuard, WorkflowReadGuardSettings};
use serde_json::json;
use std::path::{Path, PathBuf};

struct Project {
    _temp: tempfile::TempDir,
    root: PathBuf,
    store: PathBuf,
    run: PathBuf,
}

fn project() -> Project {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    let store = root.join(".archon/workflows");
    let run = store.join("run-1");
    std::fs::create_dir_all(run.join("v2/branches/call-1-item-1")).unwrap();
    std::fs::create_dir_all(run.join("artifacts")).unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    Project {
        _temp: temp,
        root,
        store,
        run,
    }
}

fn scope_at(project: &Project, working_root: &Path) -> RunStoreScope {
    RunStoreScope::new(
        Some(&project.run.display().to_string()),
        Some(&project.store.display().to_string()),
        Some(&working_root.display().to_string()),
    )
}

fn write(scope: RunStoreScope, path: &str) -> Option<String> {
    WorkflowReadGuard::from_settings(&WorkflowReadGuardSettings::default())
        .with_run_store(scope)
        .before_tool("Write", &json!({ "file_path": path, "content": "{}" }))
}

#[test]
fn a_project_root_workspace_does_not_exempt_the_store_beneath_it() {
    let project = project();
    let scope = scope_at(&project, &project.root);

    for record in [
        project.run.join("v2/branches/call-1-item-1/record.json"),
        project.run.join("state.json"),
        project.store.join("run-0/v2/results/call.json"),
    ] {
        assert!(
            scope.holds_host_records(&record),
            "{} is host bookkeeping even when the call runs at the project root",
            record.display()
        );
        assert!(write(scope.clone(), &record.display().to_string()).is_some());
    }
    assert!(
        write(scope.clone(), ".archon/workflows/run-1/state.json").is_some(),
        "a relative spelling resolves against the project root and is refused too"
    );
}

#[test]
fn a_project_root_workspace_keeps_its_own_files_and_the_artifact_area() {
    let project = project();
    let scope = scope_at(&project, &project.root);

    assert_eq!(write(scope.clone(), "src/lib.rs"), None);
    assert_eq!(
        write(
            scope.clone(),
            &project.root.join("src/lib.rs").display().to_string()
        ),
        None
    );
    assert_eq!(
        write(
            scope.clone(),
            &project
                .run
                .join("artifacts/report.json")
                .display()
                .to_string()
        ),
        None
    );
}

#[test]
fn a_workspace_inside_the_store_is_still_exempt() {
    let project = project();
    let worktree = project.run.join("v2/worktrees/call-1/item-1");
    std::fs::create_dir_all(&worktree).unwrap();
    let scope = scope_at(&project, &worktree);

    assert_eq!(
        write(scope, &worktree.join("src/lib.rs").display().to_string()),
        None
    );
}

/// The host's admission rule is the only widening, and it widens exactly by
/// what it accepts: the admitted file is writable, its neighbours are not.
#[test]
fn a_path_the_host_admits_is_writable_and_nothing_beside_it() {
    let project = project();
    let admitted = project.store.join("run-1-gap-audit.json");
    let only = admitted.clone();
    let scope = scope_at(&project, &project.root)
        .with_admitted_writes(super::AdmittedWrites::new(move |path| path == only));

    assert_eq!(write(scope.clone(), &admitted.display().to_string()), None);
    assert!(
        write(
            scope.clone(),
            &project.store.join("run-1-other.json").display().to_string()
        )
        .is_some()
    );
    assert!(
        write(
            scope,
            &project.run.join("v2/branches/x.json").display().to_string()
        )
        .is_some()
    );
}
