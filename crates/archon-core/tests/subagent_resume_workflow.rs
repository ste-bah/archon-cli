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
