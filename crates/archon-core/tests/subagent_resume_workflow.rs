//! A workflow owns the confinement of the agents it calls (#241). Its
//! validation repair continues the stored context when that context can run
//! exactly. When the context is gone (a restart, a collection) or its worktree
//! was removed at completion, the repair is a clean re-run of the workflow's
//! call: the workflow's confinement, the workflow's prompt, and nothing the
//! earlier agent did. Any other difference refuses the repair.
#[path = "support/boundary_harness.rs"]
mod harness;
#[path = "support/resume_memory_harness.rs"]
mod memory_harness;
use archon_core::agent::AgentConfig;
use archon_tools::isolation::{AutoIsolation, IsolationTier};
use archon_tools::subagent_executor::SubagentExecutor;
use archon_tools::subagent_session::{CompletedHistory, SubagentSession, scope};
use archon_tools::tool::ToolContext;
use harness::*;
use memory_harness::*;
use std::path::Path;

/// The context a workflow run hands every agent call it makes.
fn workflow_parent(root: &Path, workspace: &Path) -> ToolContext {
    ToolContext {
        write_roots: vec![workspace.to_path_buf()],
        denied_directory_names: vec!["secret".into()],
        run_store: Some(Default::default()),
        ..parent(root, &[])
    }
}

fn workflow_request(workspace: &Path) -> archon_tools::subagent_request::SubagentRequest {
    let mut request = request(workspace, None, vec![]);
    request.write_roots = vec![workspace.display().to_string()];
    request.allowed_tools.push("ContextProbe".into());
    request
}

/// One call of the workflow as agent `id`, first or repair.
async fn call(
    host: &Host,
    id: &str,
    history: &CompletedHistory,
    continuing: bool,
    root: &Path,
    workspace: &Path,
) -> Result<String, archon_tools::subagent_executor::ExecutorError> {
    scope(
        SubagentSession {
            agent_id: id.into(),
            history: history.clone(),
            continuing,
        },
        host.spawn(id, workflow_request(workspace), workflow_parent(root, workspace)),
    )
    .await
}

/// The repair ran under the workflow's confinement and saw only its prompt.
fn assert_clean_rerun(host: &Host, outside: &Path, write_call: usize) {
    let contexts = host.contexts.lock().unwrap();
    let probe = contexts.last().expect("the repair never ran");
    assert_eq!(probe.denied_directory_names, vec!["secret".to_string()]);
    assert!(probe.run_store.is_some(), "the workflow's run store was lost");
    assert!(host.outcome(write_call).is_error, "the workflow's write roots were lost");
    assert!(!outside.exists());
    let replayed = host
        .last_messages()
        .iter()
        .any(|message| message["role"] == "assistant" && message["content"] == "done");
    assert!(!replayed, "the earlier agent's turns were replayed");
}

#[tokio::test]
async fn a_repair_after_a_restart_is_a_clean_rerun_under_the_workflow() {
    let (_t, root) = real_temp();
    let workspace = dir(&root, "workspace");
    let outside = root.join("outside.txt");
    let id = "run-1-0-coder-restart";
    let history = CompletedHistory::default();
    let before = Host::new(&root, "workflow-restart", vec![STOP]);
    call(&before, id, &history, false, &root, &workspace)
        .await
        .unwrap();
    // A new process: a new manager and executor, the call's history kept.
    let after = Host::new(
        &root,
        "workflow-restart",
        vec![("ContextProbe", serde_json::json!({})), write(&outside, "x"), STOP],
    );
    let result = call(&after, id, &history, true, &root, &workspace).await;
    assert!(result.is_ok(), "the repair stopped the run: {result:?}");
    assert_clean_rerun(&after, &outside, 1);
}

#[tokio::test]
async fn a_repair_whose_context_was_collected_is_a_clean_rerun_under_the_workflow() {
    let (_t, root) = real_temp();
    let workspace = dir(&root, "workspace");
    let outside = root.join("outside.txt");
    let id = "run-1-0-coder-collected";
    let history = CompletedHistory::default();
    let host = Host::new(
        &root,
        "workflow-collected",
        vec![STOP, ("ContextProbe", serde_json::json!({})), write(&outside, "x"), STOP],
    );
    call(&host, id, &history, false, &root, &workspace)
        .await
        .unwrap();
    {
        let mut manager = host.manager.lock().await;
        for index in 0..300 {
            let other = format!("temporary-{index}");
            manager
                .register_with_id(other.clone(), request(&root, None, vec![]))
                .unwrap();
            manager.complete(&other, "done".into()).unwrap();
            manager.cleanup_agent(&other);
        }
        assert!(manager.get_status(id).is_none());
    }
    let result = call(&host, id, &history, true, &root, &workspace).await;
    assert!(result.is_ok(), "the repair stopped the run: {result:?}");
    assert_clean_rerun(&host, &outside, 2);
}

#[tokio::test]
async fn a_repair_after_its_clean_worktree_was_removed_gets_a_new_placed_worktree() {
    let (_t, root) = real_temp();
    let repo = checkout(&root);
    let outside = root.join("outside.txt");
    let id = "run-1-0-coder-worktree";
    let history = CompletedHistory::default();
    let host = Host::with_config(
        &root,
        "workflow-worktree",
        vec![
            ("ContextProbe", serde_json::json!({})),
            STOP,
            ("ContextProbe", serde_json::json!({})),
            write(&outside, "x"),
            STOP,
        ],
        AgentConfig {
            subagent_auto_isolation: AutoIsolation::Always,
            ..Default::default()
        },
    );
    call(&host, id, &history, false, &root, &repo).await.unwrap();
    let first = host.contexts.lock().unwrap()[0].working_dir.clone();
    // The first call made no change, so its completion removes the worktree.
    host.executor
        .on_visible_complete(id.into(), Ok("unparsable".into()), false)
        .await;
    assert!(!first.exists(), "completion kept the clean worktree");
    // Cleanup deletes the branch through the process's own checkout, which
    // in a test is not this fixture; delete it here as it would be there.
    let branch = format!("archon/subagent-{id}");
    for args in [vec!["worktree", "prune"], vec!["branch", "-D", &branch]] {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(&args)
            .status()
            .unwrap();
        assert!(status.success(), "{args:?}");
    }
    let result = call(&host, id, &history, true, &root, &repo).await;
    assert!(result.is_ok(), "the repair stopped the run: {result:?}");
    let contexts = host.contexts.lock().unwrap();
    let repair = contexts.last().unwrap();
    assert_ne!(repair.working_dir, repo, "the repair ran in the source checkout");
    assert!(repair.working_dir.is_dir(), "the repair has no worktree");
    assert!(!repair.sealed_repositories.is_empty(), "the placement seal was lost");
    drop(contexts);
    assert_clean_rerun(&host, &outside, 3);
}

#[tokio::test]
async fn a_repair_under_a_lowered_tier_cap_is_refused_not_run_weaker() {
    let (_t, root) = real_temp();
    let repo = checkout(&root);
    let id = "run-1-0-coder-capped";
    let history = CompletedHistory::default();
    let first = Host::with_config(
        &root,
        "workflow-cap",
        vec![STOP],
        AgentConfig {
            subagent_auto_isolation: AutoIsolation::Always,
            ..Default::default()
        },
    );
    call(&first, id, &history, false, &root, &repo).await.unwrap();
    let capped = Host::with_manager(
        &root,
        "workflow-cap",
        vec![STOP],
        AgentConfig {
            subagent_isolation_max_tier: IsolationTier::Shared,
            ..Default::default()
        },
        first.manager.clone(),
    );
    let refusal = call(&capped, id, &history, true, &root, &repo)
        .await
        .expect_err("a repair ran on a lower rung than its history")
        .to_string();
    assert!(refusal.contains("isolation_max_tier"), "{refusal}");
    assert_eq!(capped.turns(), 0, "the refused repair still ran");
}
