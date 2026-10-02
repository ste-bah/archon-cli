//! Issue-227: a shell whose caller REQUIRES the OS write boundary is refused
//! on a host that cannot apply one, never run unbounded; one whose caller
//! only had a best-effort bound runs as before; and the harness's watchdog
//! never reaches a command's output.
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::json;

use super::bash_write_sandbox::{force_snapshot_for_tests, unavailable_for_tests};
use super::*;
use crate::tool::ToolContext;
use crate::workflow_read_guard::{
    DeclaredTargetScope, HostWriteBoundary, ReadOnlyBoundaryScope, RunStoreScope,
    WorkflowReadGuard, WorkflowReadGuardSettings,
};

struct Layout {
    _base: tempfile::TempDir,
    project: PathBuf,
    checkout: PathBuf,
    run: PathBuf,
    worktree: PathBuf,
}

fn layout() -> Layout {
    let base = tempfile::tempdir().unwrap();
    let root = base
        .path()
        .canonicalize()
        .map(archon_shell::paths::plain)
        .unwrap();
    let project = root.join("project");
    let checkout = root.join("checkout");
    let run = project.join(".archon/workflows/wf-synthetic");
    let worktree = run.join("v2/worktrees/item-a/item-a-0");
    for dir in [&worktree, &checkout] {
        std::fs::create_dir_all(dir).unwrap();
    }
    Layout {
        _base: base,
        project,
        checkout,
        run,
        worktree,
    }
}

fn s(path: &Path) -> String {
    path.display().to_string()
}

fn ctx(guard: WorkflowReadGuard, working: &Path) -> ToolContext {
    ToolContext {
        working_dir: working.to_path_buf(),
        session_id: "bounded-shell-session".into(),
        workflow_read_guard: Some(Arc::new(guard)),
        ..ToolContext::default()
    }
}

fn run_store(layout: &Layout, working: &Path) -> RunStoreScope {
    let store = layout.run.parent().unwrap();
    RunStoreScope::new(layout.run.to_str(), store.to_str(), working.to_str())
}

/// A path as a shell command names it: forward slashes, which MSYS bash and
/// Python both accept on Windows. A backslashed path inside the `-c` command
/// text is mangled on its way into MSYS bash (Issue-234). A no-op elsewhere.
fn sh(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

async fn bash(ctx: &ToolContext, command: &str) -> crate::tool::ToolResult {
    BashTool::default()
        .execute(json!({"command": command}), ctx)
        .await
}

#[tokio::test]
async fn a_read_only_shell_on_a_host_with_no_boundary_is_refused_not_run() {
    let layout = layout();
    let scope = ReadOnlyBoundaryScope::new(
        &s(&layout.checkout),
        &[s(&layout.checkout), s(&layout.project)],
        &[],
    )
    .unwrap();
    let guard = WorkflowReadGuard::shell_only(&WorkflowReadGuardSettings::default())
        .with_read_only_boundary(scope)
        .with_run_store(run_store(&layout, &layout.checkout));
    let ctx = ctx(guard, &layout.checkout);
    let marker = layout.checkout.join("ran.txt");
    unavailable_for_tests(true);
    let result = bash(&ctx, &format!("printf ran > {}", marker.display())).await;
    unavailable_for_tests(false);
    assert!(result.is_error, "{}", result.content);
    assert!(
        result.content.contains("NOT run")
            && result.content.contains(std::env::consts::OS)
            && result.content.contains("a test's stand-in"),
        "{}",
        result.content
    );
    assert!(!marker.exists(), "the command never started");
}

/// An isolated write branch's shell was always best effort without a
/// boundary (its landing re-checks what it touched): it still runs.
#[tokio::test]
async fn a_write_branch_shell_on_a_host_with_no_boundary_still_runs() {
    let layout = layout();
    let guard = WorkflowReadGuard::new(40, 20, false, false)
        .with_declared_targets(
            DeclaredTargetScope::new(&["own.txt".to_string()], layout.worktree.to_str())
                .in_isolated_worktree(true)
                .with_write_boundary(HostWriteBoundary::new(
                    &[s(&layout.project), s(&layout.checkout)],
                    &[],
                )),
        )
        .with_run_store(run_store(&layout, &layout.worktree));
    let ctx = ctx(guard, &layout.worktree);
    unavailable_for_tests(true);
    let result = bash(&ctx, "printf ok > own.txt").await;
    unavailable_for_tests(false);
    assert!(!result.is_error, "{}", result.content);
    assert_eq!(
        std::fs::read_to_string(layout.worktree.join("own.txt")).unwrap(),
        "ok"
    );
}

/// Issue-234/213: on the host-snapshot path (Windows, no kernel boundary) a
/// write branch's shell cannot leave the canonical checkout changed. It writes
/// a tracked file from its worktree; the host restores it and fails the call,
/// and the checkout reads back as it was.
#[tokio::test]
async fn a_write_branch_shell_cannot_leave_the_canonical_checkout_changed() {
    let layout = layout();
    let tracked = layout.checkout.join("src/lib.rs");
    std::fs::create_dir_all(tracked.parent().unwrap()).unwrap();
    std::fs::write(&tracked, "original").unwrap();
    let guard = WorkflowReadGuard::new(40, 20, false, false)
        .with_declared_targets(
            DeclaredTargetScope::new(&["own.txt".to_string()], layout.worktree.to_str())
                .in_isolated_worktree(true)
                .with_write_boundary(HostWriteBoundary::new(
                    &[s(&layout.project), s(&layout.checkout)],
                    &[],
                )),
        )
        .with_run_store(run_store(&layout, &layout.worktree));
    let ctx = ctx(guard, &layout.worktree);
    force_snapshot_for_tests(true);
    let result = bash(&ctx, &format!("printf hacked > {}", sh(&tracked))).await;
    // Its own worktree is its work and lands normally.
    let own = bash(&ctx, "printf ok > own.txt").await;
    force_snapshot_for_tests(false);
    assert!(
        result.is_error,
        "the checkout write must fail: {}",
        result.content
    );
    // Read the canonical checkout back: unchanged.
    assert_eq!(std::fs::read_to_string(&tracked).unwrap(), "original");
    assert!(!own.is_error, "{}", own.content);
    assert_eq!(
        std::fs::read_to_string(layout.worktree.join("own.txt")).unwrap(),
        "ok"
    );
}

/// bash reports a background job killed by a signal on stderr; the watcher
/// the wrapper starts is killed at every exit and must never be reported.
#[tokio::test]
async fn the_watchdog_never_reaches_the_output() {
    let ctx = ToolContext {
        working_dir: std::env::temp_dir(),
        session_id: "watchdog-quiet".into(),
        ..ToolContext::default()
    };
    for _ in 0..8 {
        let result = bash(&ctx, "printf '%s\\n' one two").await;
        assert!(!result.is_error, "{}", result.content);
        assert!(
            !result.content.contains("__archon") && !result.content.contains("Killed"),
            "{}",
            result.content
        );
        assert_eq!(result.content.trim_end(), "one\ntwo", "{}", result.content);
    }
}
