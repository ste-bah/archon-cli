//! A workflow's validation repair continues its call's stored context exactly
//! or is refused (#241). Nothing is rebuilt for it: not after a restart, and
//! not after its worktree was removed at completion.
#[path = "support/boundary_harness.rs"]
mod harness;
#[path = "support/resume_memory_harness.rs"]
mod memory_harness;
use archon_core::agent::AgentConfig;
use archon_tools::isolation::AutoIsolation;
use archon_tools::subagent_executor::SubagentExecutor;
use harness::*;
use memory_harness::*;

#[tokio::test]
async fn a_repair_after_a_restart_is_refused_and_runs_nothing() {
    let (_t, root) = temp();
    let workspace = dir(&root, "workspace");
    let id = "run-1-0-coder-restart";
    let before = Host::new(&root, "workflow-restart", vec![STOP]);
    before
        .spawn(id, request(&workspace, None, vec![]), parent(&root, &[]))
        .await
        .unwrap();
    let history = before.histories.lock().unwrap()[id].clone();
    // A new process: a new manager and executor, the call's history kept.
    let after = Host::new(&root, "workflow-restart", vec![STOP]);
    after.histories.lock().unwrap().insert(id.into(), history);
    let refusal = after
        .repair(id, request(&workspace, None, vec![]), parent(&root, &[]))
        .await
        .expect_err("a repair ran without its stored context")
        .to_string();
    assert!(refusal.contains(&unknown(id)), "{refusal}");
    assert_eq!(after.turns(), 0, "the refused repair still ran");
}

#[tokio::test]
async fn a_repair_after_its_clean_worktree_was_removed_is_refused() {
    let (_t, root) = temp();
    let repo = checkout(&root);
    let id = "run-1-0-coder-worktree";
    let host = Host::with_config(
        &root,
        "workflow-worktree",
        vec![STOP, STOP],
        AgentConfig {
            subagent_auto_isolation: AutoIsolation::Always,
            ..Default::default()
        },
    );
    host.spawn(id, request(&repo, None, vec![]), parent(&root, &[]))
        .await
        .unwrap();
    // The call made no change, so its completion removes the worktree.
    host.executor
        .on_visible_complete(id.into(), Ok("unparsable".into()), false)
        .await;
    let refusal = host
        .repair(id, request(&repo, None, vec![]), parent(&root, &[]))
        .await
        .expect_err("a repair ran without the worktree it was placed in")
        .to_string();
    assert!(
        refusal.contains(id) && refusal.contains("working directory"),
        "{refusal}"
    );
    assert_eq!(host.turns(), 1, "the refused repair still ran");
}

/// One worktree-isolated call that records where it was placed.
async fn placed_call(root: &std::path::Path, id: &str) -> (Host, std::path::PathBuf) {
    let repo = checkout(root);
    let host = Host::with_config(
        root,
        "workflow-worktree-identity",
        vec![("ContextProbe", serde_json::json!({})), STOP, STOP],
        AgentConfig {
            subagent_auto_isolation: AutoIsolation::Always,
            ..Default::default()
        },
    );
    let mut call = request(&repo, None, vec![]);
    call.allowed_tools.push("ContextProbe".into());
    host.spawn(id, call, parent(root, &[])).await.unwrap();
    let placed = host.contexts.lock().unwrap()[0].working_dir.clone();
    assert_ne!(placed, repo, "the call was not placed in a worktree");
    (host, placed)
}

async fn refused_repair(host: &Host, root: &std::path::Path, id: &str) -> String {
    let refusal = host
        .repair(id, request(root, None, vec![]), parent(root, &[]))
        .await
        .expect_err("a repair ran in a directory that is not its worktree")
        .to_string();
    assert_eq!(host.turns(), 2, "the refused repair still ran");
    refusal
}

#[tokio::test]
async fn a_plain_directory_recreated_at_the_worktree_path_refuses_the_repair() {
    let (_t, root) = temp();
    let id = "run-1-0-coder-recreated";
    let (host, placed) = placed_call(&root, id).await;
    host.executor
        .on_visible_complete(id.into(), Ok("unparsable".into()), false)
        .await;
    assert!(!placed.exists(), "completion kept the clean worktree");
    std::fs::create_dir_all(&placed).unwrap();
    let refusal = refused_repair(&host, &root, id).await;
    assert!(
        refusal.contains(id) && refusal.contains("no longer"),
        "{refusal}"
    );
}

#[tokio::test]
async fn a_worktree_whose_git_metadata_is_gone_refuses_the_repair() {
    let (_t, root) = temp();
    let id = "run-1-0-coder-orphaned";
    let (host, placed) = placed_call(&root, id).await;
    let pointer = std::fs::read_to_string(placed.join(".git")).unwrap();
    let git_dir = pointer.trim().strip_prefix("gitdir:").unwrap().trim();
    std::fs::remove_dir_all(git_dir).unwrap();
    assert!(placed.is_dir());
    let refusal = refused_repair(&host, &root, id).await;
    assert!(
        refusal.contains(id) && refusal.contains("working directory is gone"),
        "{refusal}"
    );
}

/// A call dispatched into a workspace the workflow supplied, with no worktree
/// of the executor's own; the workspace is then replaced at the same path.
async fn replaced_supplied_workspace(root: &std::path::Path, workspace: &std::path::Path) {
    let id = "run-1-0-coder-supplied";
    let host = Host::new(root, "workflow-supplied", vec![STOP, STOP]);
    host.spawn(id, request(workspace, None, vec![]), parent(root, &[]))
        .await
        .unwrap();
    std::fs::remove_dir_all(workspace).unwrap();
    std::fs::create_dir_all(workspace).unwrap();
    let refusal = host
        .repair(id, request(workspace, None, vec![]), parent(root, &[]))
        .await
        .expect_err("a repair ran in a replaced workspace")
        .to_string();
    assert!(
        refusal.contains(id) && refusal.contains("no longer"),
        "{refusal}"
    );
    assert_eq!(host.turns(), 1, "the refused repair still ran");
}

#[tokio::test]
async fn a_replaced_supplied_worktree_refuses_the_repair() {
    let (_t, root) = temp();
    let repo = checkout(&root);
    let supplied = root.join("supplied");
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(&repo)
        .args(["worktree", "add", "-q", "-b", "branch-1"])
        .arg(&supplied)
        .status()
        .unwrap();
    assert!(status.success());
    replaced_supplied_workspace(&root, &supplied).await;
}

#[tokio::test]
async fn a_replaced_supplied_plain_workspace_refuses_the_repair() {
    let (_t, root) = temp();
    let workspace = dir(&root, "workspace");
    replaced_supplied_workspace(&root, &workspace).await;
}
