//! The read side of the run-store boundary: a recursive walk does not descend
//! the host's bookkeeping, and still reaches the two trees the agent owns
//! inside the run directory.
use super::RunStoreScope;
use crate::glob_tool::GlobTool;
use crate::grep::GrepTool;
use crate::tool::{Tool, ToolContext};
use serde_json::json;

struct Tree {
    _temp: tempfile::TempDir,
    project: std::path::PathBuf,
    run: std::path::PathBuf,
    worktree: std::path::PathBuf,
}

/// A project holding ordinary source, an accumulated run store, and inside the
/// current run both the trees an agent owns.
fn tree() -> Tree {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let run = project.join(".archon/workflows/run-1");
    let worktree = run.join("v2/worktrees/call-1/item-1");
    let files = [
        (project.join("crates/thing/src/lib.rs"), "fn needle() {}"),
        (run.join("v2/branches/call-1-item-1/outcome.json"), "needle"),
        (run.join("v2/results/call-1.json"), "needle"),
        (run.join("state.json"), "needle"),
        // An earlier run, the bulk of what a walk would traverse.
        (
            project.join(".archon/workflows/run-0/v2/results/old.json"),
            "needle",
        ),
        (run.join("artifacts/report.json"), "needle"),
        (worktree.join("src/lib.rs"), "fn needle() {}"),
    ];
    for (path, body) in files {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }
    Tree {
        _temp: temp,
        project,
        run,
        worktree,
    }
}

fn ctx(tree: &Tree) -> ToolContext {
    ToolContext {
        working_dir: tree.project.clone(),
        run_store: Some(RunStoreScope::new(
            Some(&tree.run.display().to_string()),
            Some(&tree.project.join(".archon/workflows").display().to_string()),
            Some(&tree.worktree.display().to_string()),
        )),
        ..ToolContext::default()
    }
}

/// Without the boundary the same walk reaches every record, so the assertions
/// below are about the boundary and not about the fixture.
fn unbounded_ctx(tree: &Tree) -> ToolContext {
    ToolContext {
        working_dir: tree.project.clone(),
        ..ToolContext::default()
    }
}

#[tokio::test]
async fn glob_skips_the_run_bookkeeping_and_keeps_what_the_agent_owns() {
    let tree = tree();
    let result = GlobTool
        .execute(json!({ "pattern": "**/*.json" }), &ctx(&tree))
        .await;
    assert!(!result.is_error, "{}", result.content);

    for host_record in [
        "v2/branches/call-1-item-1/outcome.json",
        "v2/results/call-1.json",
        "state.json",
    ] {
        assert!(
            !result.content.contains(host_record),
            "{host_record} is the host's: {}",
            result.content
        );
    }
    assert!(
        !result.content.contains("run-0"),
        "past runs are never walked: {}",
        result.content
    );
    assert!(
        result.content.contains("artifacts/report.json"),
        "the run's artifact area stays visible: {}",
        result.content
    );
}

#[tokio::test]
async fn glob_without_the_boundary_walks_straight_into_the_records() {
    let tree = tree();
    let result = GlobTool
        .execute(json!({ "pattern": "**/*.json" }), &unbounded_ctx(&tree))
        .await;
    assert!(!result.is_error, "{}", result.content);
    assert!(
        result.content.contains("v2/results/call-1.json") && result.content.contains("run-0"),
        "the fixture must be reachable at all: {}",
        result.content
    );
}

#[tokio::test]
async fn grep_skips_the_run_bookkeeping_and_keeps_the_worktree() {
    let tree = tree();
    let result = GrepTool
        .execute(json!({ "pattern": "needle" }), &ctx(&tree))
        .await;
    assert!(!result.is_error, "{}", result.content);

    assert!(
        !result.content.contains("v2/results") && !result.content.contains("v2/branches"),
        "the host's records are not searched: {}",
        result.content
    );
    assert!(
        !result.content.contains("run-0"),
        "past runs are never searched: {}",
        result.content
    );
    assert!(
        result.content.contains("crates/thing/src/lib.rs"),
        "ordinary source is still searched: {}",
        result.content
    );
}

/// The agent's own workspace sits inside the run directory, and a search
/// rooted there must still work — the boundary must never take away the tree
/// the agent was given to work in. Searched by explicit path because Grep
/// prunes every dot-directory on its own, so a search from the project root
/// never descends into the run store at all.
#[tokio::test]
async fn grep_inside_the_branch_worktree_is_untouched_by_the_boundary() {
    let tree = tree();
    let result = GrepTool
        .execute(
            json!({ "pattern": "needle", "path": tree.worktree.display().to_string() }),
            &ctx(&tree),
        )
        .await;
    assert!(!result.is_error, "{}", result.content);
    assert!(
        result.content.contains("src/lib.rs"),
        "the agent's own workspace is still searched: {}",
        result.content
    );
}

/// And a search rooted at a sibling run — the accumulated history — finds
/// nothing, because none of it is the agent's to read through.
#[tokio::test]
async fn glob_rooted_in_a_finished_run_matches_nothing() {
    let tree = tree();
    let result = GlobTool
        .execute(
            json!({
                "pattern": "**/*.json",
                "path": tree.project.join(".archon/workflows/run-0").display().to_string(),
            }),
            &ctx(&tree),
        )
        .await;
    assert!(!result.is_error, "{}", result.content);
    assert!(
        !result.content.contains("old.json"),
        "a finished run is not walked even when named: {}",
        result.content
    );
}
