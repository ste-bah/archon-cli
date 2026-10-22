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

#[tokio::test]
async fn grep_skips_the_run_bookkeeping_and_keeps_the_worktree() {
    let tree = tree();
    let result = GrepTool
        .execute(json!({ "pattern": "needle" }), &ctx(&tree))
        .await;
    assert!(!result.is_error, "{}", result.content);

    assert!(
        !result.content.replace('\\', "/").contains("v2/results")
            && !result.content.replace('\\', "/").contains("v2/branches"),
        "the host's records are not searched: {}",
        result.content
    );
    assert!(
        !result.content.replace('\\', "/").contains("run-0"),
        "past runs are never searched: {}",
        result.content
    );
    assert!(
        result
            .content
            .replace('\\', "/")
            .contains("crates/thing/src/lib.rs"),
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
        result.content.replace('\\', "/").contains("src/lib.rs"),
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
        !result.content.replace('\\', "/").contains("old.json"),
        "a finished run is not walked even when named: {}",
        result.content
    );
}

#[tokio::test]
async fn glob_prunes_hidden_harness_state_but_allows_an_explicit_target() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    for n in 0..64 {
        let file = project
            .join(".archon/workflows")
            .join(format!("run-{n}/results/match.json"));
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, "{}").unwrap();
    }
    let ctx = ToolContext {
        working_dir: project.clone(),
        ..ToolContext::default()
    };
    let hidden = GlobTool
        .execute(json!({"pattern": "**/*.json"}), &ctx)
        .await;
    assert!(!hidden.is_error, "{}", hidden.content);
    assert!(!hidden.content.contains(".archon"), "{}", hidden.content);

    let targeted = GlobTool
        .execute(
            json!({"pattern": "**/.archon/workflows/run-0/**/*.json"}),
            &ctx,
        )
        .await;
    assert!(!targeted.is_error, "{}", targeted.content);
    assert!(
        targeted.content.contains("match.json"),
        "{}",
        targeted.content
    );
}

#[tokio::test]
async fn glob_does_not_target_a_nested_state_root_from_an_unrelated_prefix() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let state_root = project.join("src/store");
    std::fs::create_dir_all(project.join("src")).unwrap();
    std::fs::write(project.join("src/ordinary.json"), "{}").unwrap();
    for n in 0..64 {
        let file = state_root.join(format!("run-{n}/match.json"));
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, "{}").unwrap();
    }
    let ctx = ToolContext {
        working_dir: project.clone(),
        ..ToolContext::default()
    };
    let matches = crate::glob_tool::bounded_glob_with_test_state_roots(
        &project,
        "src/**/*.json",
        &ctx,
        vec![state_root.clone()],
    )
    .await
    .unwrap();

    assert!(
        matches
            .iter()
            .any(|path| path.ends_with("src/ordinary.json"))
    );
    assert!(
        matches.iter().all(|path| !path.starts_with(&state_root)),
        "nested state files must be pruned: {matches:?}"
    );
}

#[tokio::test]
async fn glob_targets_a_nested_state_root_when_its_path_is_explicit() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let state_root = project.join("src/store");
    for n in 0..64 {
        let file = state_root.join(format!("run-{n}/match.json"));
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, "{}").unwrap();
    }
    let ctx = ToolContext {
        working_dir: project.clone(),
        ..ToolContext::default()
    };
    let matches = crate::glob_tool::bounded_glob_with_test_state_roots(
        &project,
        "src/store/**/*.json",
        &ctx,
        vec![state_root.clone()],
    )
    .await
    .unwrap();

    assert!(
        matches.iter().any(|path| path.starts_with(&state_root)),
        "explicitly targeted state files should be found: {matches:?}"
    );
}

#[tokio::test]
async fn glob_from_archon_directory_skips_workflow_run_stores() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let archon = project.join(".archon");
    let state_root = archon.join("workflows");
    std::fs::create_dir_all(&archon).unwrap();
    std::fs::write(archon.join("settings.json"), "{}").unwrap();
    for run in ["run-0", "run-1"] {
        let file = state_root.join(run).join("results/match.json");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, "{}").unwrap();
    }
    let ctx = ToolContext {
        working_dir: project.clone(),
        ..ToolContext::default()
    };

    let result = GlobTool
        .execute(json!({"path": archon, "pattern": "**/*.json"}), &ctx)
        .await;
    assert!(!result.is_error, "{}", result.content);

    assert!(
        result.content.contains("settings.json"),
        "{}",
        result.content
    );
    assert!(
        !result.content.contains("workflows"),
        "workflow run stores must be pruned from a search rooted at .archon: {}",
        result.content
    );
}

#[tokio::test]
async fn glob_from_project_directory_skips_workflow_run_stores() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let state_root = project.join(".archon/workflows");
    std::fs::create_dir_all(project.join("src")).unwrap();
    std::fs::write(project.join("src/main.rs"), "fn main() {}").unwrap();
    for run in ["run-0", "run-1"] {
        let file = state_root.join(run).join("results/match.json");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, "{}").unwrap();
    }
    let ctx = ToolContext {
        working_dir: project.clone(),
        ..ToolContext::default()
    };

    let result = GlobTool
        .execute(json!({"path": project, "pattern": "**/*"}), &ctx)
        .await;
    assert!(!result.is_error, "{}", result.content);

    assert!(result.content.contains("main.rs"), "{}", result.content);
    assert!(
        !result.content.contains("workflows"),
        "workflow run stores must be pruned from a project search: {}",
        result.content
    );
}

#[tokio::test]
async fn glob_rooted_inside_a_run_store_lists_that_run() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let state_root = project.join(".archon/workflows");
    let own_run = state_root.join("run-1");
    let own_file = own_run.join("artifacts/report.json");
    let other_file = state_root.join("run-0/artifacts/old.json");
    std::fs::create_dir_all(own_file.parent().unwrap()).unwrap();
    std::fs::create_dir_all(other_file.parent().unwrap()).unwrap();
    std::fs::write(&own_file, "{}").unwrap();
    std::fs::write(&other_file, "{}").unwrap();
    let ctx = ToolContext {
        working_dir: project,
        ..ToolContext::default()
    };

    let result = GlobTool
        .execute(json!({"path": own_run, "pattern": "**/*.json"}), &ctx)
        .await;
    assert!(!result.is_error, "{}", result.content);

    assert!(
        result.content.contains("artifacts/report.json"),
        "a search rooted inside one run must still list its contents: {}",
        result.content
    );
}

#[tokio::test]
async fn glob_honors_gitignore_directories() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    std::fs::create_dir_all(project.join("ignored/nested")).unwrap();
    std::fs::create_dir_all(project.join("visible")).unwrap();
    std::fs::write(project.join(".gitignore"), "ignored/\n").unwrap();
    std::fs::write(project.join("ignored/nested/match.txt"), "hidden").unwrap();
    std::fs::write(project.join("visible/match.txt"), "visible").unwrap();
    let ctx = ToolContext {
        working_dir: project,
        ..ToolContext::default()
    };
    let result = GlobTool.execute(json!({"pattern": "**/*.txt"}), &ctx).await;
    assert!(!result.is_error, "{}", result.content);
    let output = result.content.replace('\\', "/");
    assert!(output.contains("visible/match.txt"), "{}", result.content);
    assert!(!output.contains("ignored"), "{}", result.content);
}
