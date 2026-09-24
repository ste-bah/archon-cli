//! The run directory is the host's, except its artifact area and the branch
//! worktree inside it. A project artifact root outside the run directory is
//! untouched by this rule.
use super::{RunStoreScope, scope_run_store};
use crate::workflow_read_guard::{WorkflowReadGuard, WorkflowReadGuardSettings};
use serde_json::json;

/// A run directory shaped like a live one, with the branch worktree the host
/// plants inside it and a project artifact root that is NOT inside it.
struct Run {
    _temp: tempfile::TempDir,
    store: std::path::PathBuf,
    run: std::path::PathBuf,
    worktree: std::path::PathBuf,
    project_artifacts: std::path::PathBuf,
}

fn live_run() -> Run {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let store = project.join(".archon/workflows");
    let run = store.join("run-1");
    let worktree = run.join("v2/worktrees/call-1/item-1");
    let project_artifacts = project.join(".archon/artifacts");
    for dir in [
        &run.join("v2/branches/call-1-item-1"),
        &run.join("artifacts"),
        &worktree,
        &project_artifacts,
    ] {
        std::fs::create_dir_all(dir).unwrap();
    }
    Run {
        _temp: temp,
        store,
        run,
        worktree,
        project_artifacts,
    }
}

fn guard(run: &Run) -> WorkflowReadGuard {
    WorkflowReadGuard::from_settings(&WorkflowReadGuardSettings::default()).with_run_store(
        RunStoreScope::new(
            Some(&run.run.display().to_string()),
            Some(&run.store.display().to_string()),
            Some(&run.worktree.display().to_string()),
        ),
    )
}

fn write(guard: &WorkflowReadGuard, path: &std::path::Path) -> Option<String> {
    guard.before_tool(
        "Write",
        &json!({ "file_path": path.display().to_string(), "content": "{}" }),
    )
}

fn bash(guard: &WorkflowReadGuard, command: &str) -> Option<String> {
    guard.before_tool("Bash", &json!({ "command": command }))
}

#[test]
fn a_write_into_the_per_branch_records_is_refused_and_says_where_to_write() {
    let run = live_run();
    let guard = guard(&run);
    let record = run
        .run
        .join("v2/branches/call-1-item-1/remediation-record.json");

    let refusal = write(&guard, &record).expect("the host's records are not a branch's to write");

    assert!(refusal.contains("run's own record directory"), "{refusal}");
    assert!(
        refusal.contains(&run.run.join("artifacts").display().to_string()),
        "the refusal must name the artifact area: {refusal}"
    );
    assert!(
        refusal.contains(&run.worktree.display().to_string()),
        "the refusal must name the agent's own workspace: {refusal}"
    );
}

#[test]
fn the_rest_of_the_run_bookkeeping_is_refused_too() {
    let run = live_run();
    let guard = guard(&run);
    for relative in [
        "state.json",
        "events.jsonl",
        "v2/results/call-1.json",
        "v2/stage-records/stage-1.json",
        "write-coordination/manifest.json",
    ] {
        assert!(
            write(&guard, &run.run.join(relative)).is_some(),
            "{relative} is host bookkeeping"
        );
    }
}

#[test]
fn the_runs_artifact_area_is_allowed() {
    let run = live_run();
    let guard = guard(&run);
    assert_eq!(write(&guard, &run.run.join("artifacts/report.json")), None);
    assert_eq!(
        write(&guard, &run.run.join("artifacts/nested/deep/report.json")),
        None
    );
}

#[test]
fn the_branchs_own_worktree_is_allowed() {
    let run = live_run();
    let guard = guard(&run);
    assert_eq!(
        write(&guard, &run.worktree.join("crates/x/src/lib.rs")),
        None
    );
    // And the worktree area as a whole, so a guard built without the call's
    // working root still cannot take an agent's workspace away.
    let blind =
        WorkflowReadGuard::from_settings(&WorkflowReadGuardSettings::default()).with_run_store(
            RunStoreScope::new(Some(&run.run.display().to_string()), None, None),
        );
    assert_eq!(
        write(
            &blind,
            &run.run.join("v2/worktrees/call-2/item-2/src/lib.rs")
        ),
        None
    );
    assert!(write(&blind, &run.run.join("v2/branches/x/rec.json")).is_some());
}

#[test]
fn a_project_artifact_root_outside_the_run_directory_is_still_writable() {
    let run = live_run();
    let guard = guard(&run);
    assert_eq!(
        write(&guard, &run.project_artifacts.join("deliverable.json")),
        None
    );
    assert_eq!(
        write(&guard, std::path::Path::new("/tmp/scratch-notes.md")),
        None
    );
}

#[test]
fn a_shell_write_into_the_records_is_refused_by_the_command_it_names() {
    let run = live_run();
    let guard = guard(&run);
    let record = run
        .run
        .join("v2/branches/call-1-item-1/rec.json")
        .display()
        .to_string();

    let refusal = bash(&guard, &format!("echo '{{}}' > {record}")).expect("shell write refused");
    assert!(
        refusal.starts_with('`'),
        "the refusal quotes the segment: {refusal}"
    );
    assert!(refusal.contains("run's own record directory"), "{refusal}");

    assert_eq!(
        bash(
            &guard,
            &format!(
                "echo '{{}}' > {}",
                run.run.join("artifacts/out.json").display()
            )
        ),
        None
    );
}

#[test]
fn a_relative_path_that_climbs_out_of_the_worktree_is_still_judged() {
    let run = live_run();
    let guard = guard(&run);
    // Relative to the worktree, which is `<run>/v2/worktrees/call-1/item-1`.
    assert!(write(&guard, std::path::Path::new("../../../branches/x/rec.json")).is_some());
    assert_eq!(write(&guard, std::path::Path::new("src/lib.rs")), None);
}

#[test]
fn a_guard_with_no_run_directory_judges_nothing() {
    let run = live_run();
    let guard =
        WorkflowReadGuard::from_settings(&WorkflowReadGuardSettings::default()).with_run_store(
            RunStoreScope::new(None, None, Some(&run.worktree.display().to_string())),
        );
    assert_eq!(
        write(&guard, &run.run.join("v2/branches/call-1-item-1/rec.json")),
        None
    );
}

#[tokio::test]
async fn the_scope_reaches_a_guard_built_inside_it() {
    let run = live_run();
    let record = run.run.join("v2/branches/call-1-item-1/rec.json");
    let refused = scope_run_store(
        RunStoreScope::new(
            Some(&run.run.display().to_string()),
            Some(&run.store.display().to_string()),
            Some(&run.worktree.display().to_string()),
        ),
        async {
            let guard = WorkflowReadGuard::from_settings(&WorkflowReadGuardSettings::default());
            write(&guard, &record).is_some()
        },
    )
    .await;
    assert!(refused, "the dispatch scope must reach the session's guard");
}

/// Every run kept beside this one is the host's too — that accumulation is
/// what makes a walk of the store expensive — and the exemptions belong to
/// the CURRENT run alone, not to a finished one.
#[test]
fn a_finished_run_beside_this_one_is_host_records_throughout() {
    let run = live_run();
    let guard = guard(&run);
    let finished = run.store.join("run-0");
    for relative in ["v2/results/old.json", "artifacts/old.json", "state.json"] {
        assert!(
            write(&guard, &finished.join(relative)).is_some(),
            "{relative} of a finished run is not this branch's to write"
        );
    }
}

/// Without a store root the boundary is this run alone. Being wrong that way
/// under-refuses; guessing a parent that is not the store would over-refuse
/// across an unrelated tree.
#[test]
fn without_a_store_root_only_this_run_is_judged() {
    let run = live_run();
    let guard = WorkflowReadGuard::from_settings(&WorkflowReadGuardSettings::default())
        .with_run_store(RunStoreScope::new(
            Some(&run.run.display().to_string()),
            None,
            Some(&run.worktree.display().to_string()),
        ));
    assert!(write(&guard, &run.run.join("state.json")).is_some());
    assert_eq!(write(&guard, &run.store.join("run-0/state.json")), None);
}
