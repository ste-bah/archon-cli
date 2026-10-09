//! Batch G (Issue-128): a READ-ONLY call's shell cannot write the project
//! root, the canonical checkout it stands in or the run store, and can still
//! read them and write the host's temp directory. A write branch's own
//! boundary is untouched by the read-only scope.
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::json;

use super::bash_write_sandbox::{WRITE_BOUNDARY_NOTE_MARKER, available};
use super::*;
use crate::tool::ToolContext;
use crate::workflow_read_guard::{
    DeclaredTargetScope, HostWriteBoundary, ReadOnlyBoundaryScope, RunStoreScope,
    WorkflowReadGuard, WorkflowReadGuardSettings, scope_read_only_boundary,
};

struct Layout {
    _base: tempfile::TempDir,
    project: PathBuf,
    checkout: PathBuf,
    store: PathBuf,
    run: PathBuf,
    worktree: PathBuf,
    spec: PathBuf,
}

/// The incident layout: a project whose tracked strategy spec a verifier
/// regenerated, the canonical checkout the verifier stood in, the run store.
fn layout() -> Layout {
    let base = tempfile::tempdir().unwrap();
    let root = base
        .path()
        .canonicalize()
        .map(archon_shell::paths::plain)
        .unwrap();
    let project = root.join("project");
    let checkout = root.join("checkout");
    let store = project.join(".archon/workflows");
    let run = store.join("wf-synthetic");
    let worktree = run.join("v2/worktrees/item-a/item-a-0");
    let spec = project.join(".archon/lab/strategies/s1/strategy-spec.json");
    for dir in [
        worktree.join("src"),
        run.join("artifacts"),
        checkout.join("src"),
        spec.parent().unwrap().to_path_buf(),
    ] {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(&spec, "{\"datasets\":[\"a\"]}").unwrap();
    std::fs::write(checkout.join("src/lib.rs"), "base").unwrap();
    Layout {
        _base: base,
        project,
        checkout,
        store,
        run,
        worktree,
        spec,
    }
}

fn s(path: &Path) -> String {
    path.display().to_string()
}

fn run_store(layout: &Layout, working: &Path) -> RunStoreScope {
    RunStoreScope::new(layout.run.to_str(), layout.store.to_str(), working.to_str())
}

/// The boundary the host stamps on a read-only call standing in `checkout`.
fn read_only_scope(layout: &Layout) -> ReadOnlyBoundaryScope {
    ReadOnlyBoundaryScope::new(
        &s(&layout.checkout),
        &[s(&layout.checkout), s(&layout.project)],
        &[],
    )
    .expect("a boundary")
}

fn ctx(guard: WorkflowReadGuard, working: &Path) -> ToolContext {
    ToolContext {
        working_dir: working.to_path_buf(),
        session_id: "read-only-boundary-session".into(),
        workflow_read_guard: Some(Arc::new(guard)),
        ..ToolContext::default()
    }
}

async fn bash(ctx: &ToolContext, command: &str) -> crate::tool::ToolResult {
    BashTool::default()
        .execute(json!({"command": command}), ctx)
        .await
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

/// A path as a shell command names it: forward slashes, which MSYS bash and
/// Python both accept on Windows. A backslashed path inside the `-c` command
/// text is mangled on its way into MSYS bash (Issue-234). A no-op elsewhere.
fn sh(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

fn regenerate(target: &Path) -> String {
    format!(
        "python3 - <<'EOF'\nimport os\np = {}\nopen(p + '.tmp', 'w').write('{{\"datasets\":[\"a\",\"b\"]}}')\nos.replace(p + '.tmp', p)\nEOF",
        serde_json::to_string(&sh(target)).unwrap()
    )
}

#[tokio::test]
async fn a_read_only_call_cannot_regenerate_a_project_input_from_its_shell() {
    let layout = layout();
    if !available() {
        eprintln!("skipped: no OS write boundary can be applied in this process");
        return;
    }
    let settings = WorkflowReadGuardSettings::default();
    let guard = WorkflowReadGuard::shell_only(&settings)
        .with_read_only_boundary(read_only_scope(&layout))
        .with_run_store(run_store(&layout, &layout.checkout));
    let ctx = ctx(guard, &layout.checkout);

    // The live write: EPERM, the file unchanged, the agent told why.
    let result = bash(&ctx, &regenerate(&layout.spec)).await;
    assert_eq!(
        read(&layout.spec),
        "{\"datasets\":[\"a\"]}",
        "{}",
        result.content
    );
    // EPERM from `sandbox-exec`, EACCES from Landlock.
    assert!(
        result.content.contains("Operation not permitted")
            || result.content.contains("Permission denied"),
        "{}",
        result.content
    );
    assert!(
        result.content.contains(WRITE_BOUNDARY_NOTE_MARKER) && result.content.contains("READ-ONLY"),
        "{}",
        result.content
    );
    assert!(!result.is_guard_refusal(), "{}", result.content);
    // Nor the checkout it stands in, the run's records or their artifacts.
    for command in [
        "printf x > src/lib.rs".to_string(),
        "printf x > new-file.txt".to_string(),
        format!("printf x > {}/state.json", sh(&layout.run)),
        format!("printf x > {}/artifacts/report.json", sh(&layout.run)),
        format!("mv {} {}.moved", sh(&layout.spec), sh(&layout.spec)),
    ] {
        let result = bash(&ctx, &command).await;
        assert!(result.is_error, "{command}: {}", result.content);
    }
    assert_eq!(read(&layout.checkout.join("src/lib.rs")), "base");
    assert!(!layout.checkout.join("new-file.txt").exists());
    assert!(!layout.run.join("state.json").exists());

    // Reads still work, and the host's temp directory is writable.
    let result = bash(&ctx, &format!("cat {} && cat src/lib.rs", sh(&layout.spec))).await;
    assert!(!result.is_error, "{}", result.content);
    assert!(result.content.contains("datasets") && result.content.contains("base"));
    let result = bash(
        &ctx,
        "printf scratch > \"$TMPDIR/archon-read-only-probe-$$\" && cat \"$TMPDIR/archon-read-only-probe-$$\" && rm \"$TMPDIR/archon-read-only-probe-$$\"",
    )
    .await;
    assert!(!result.is_error, "{}", result.content);
    assert!(result.content.contains("scratch"), "{}", result.content);
}

/// The boundary reaches a read-only guard built inside the dispatch scope,
/// as the live pipeline builds it; a write-capable guard built in the same
/// scope ignores it, so a write branch keeps writing its own worktree.
#[tokio::test]
async fn the_scope_reaches_read_only_guards_only() {
    let layout = layout();
    let settings = WorkflowReadGuardSettings::default();
    let (read_only, write_capable) =
        scope_read_only_boundary(Some(read_only_scope(&layout)), async {
            (
                WorkflowReadGuard::shell_only(&settings),
                WorkflowReadGuard::from_settings(&settings),
            )
        })
        .await;
    let paths = read_only
        .boundary_paths()
        .expect("a read-only call is bounded");
    assert!(paths.read_only);
    assert!(paths.refuses(&layout.spec));
    assert!(paths.refuses(&layout.checkout.join("src/lib.rs")));
    assert!(write_capable.boundary_paths().is_none());
    assert!(
        WorkflowReadGuard::shell_only(&settings)
            .boundary_paths()
            .is_none(),
        "unscoped: nothing stamped"
    );

    if !available() {
        return;
    }
    let branch = WorkflowReadGuard::from_settings(&settings)
        .with_declared_targets(
            DeclaredTargetScope::new(&["src/lib.rs".to_string()], layout.worktree.to_str())
                .in_isolated_worktree(true)
                .with_write_boundary(HostWriteBoundary::new(
                    &[s(&layout.project), s(&layout.checkout)],
                    &[],
                )),
        )
        .with_run_store(run_store(&layout, &layout.worktree));
    let ctx = ctx(branch, &layout.worktree);
    let result = bash(&ctx, "printf fixed > src/lib.rs && cat src/lib.rs").await;
    assert!(!result.is_error, "{}", result.content);
    assert_eq!(read(&layout.worktree.join("src/lib.rs")), "fixed");
    let result = bash(&ctx, &regenerate(&layout.spec)).await;
    assert_eq!(
        read(&layout.spec),
        "{\"datasets\":[\"a\"]}",
        "{}",
        result.content
    );
}
