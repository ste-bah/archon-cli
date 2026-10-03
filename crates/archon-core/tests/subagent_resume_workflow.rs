//! A workflow owns the confinement of the agents it calls. Its agent calls
//! never depend on another process's memory: after a restart a call starts a
//! new agent from the workflow's own definition, and a validation repair
//! whose stored context is gone does the same, with its in-memory history.
#[path = "support/boundary_harness.rs"]
mod harness;
#[path = "support/resume_memory_harness.rs"]
mod memory_harness;
use archon_tools::subagent_executor::SubagentExecutor;
use archon_tools::subagent_session::{CompletedHistory, SubagentSession, scope};
use archon_tools::tool::ToolContext;
use harness::*;
use memory_harness::*;

/// The context a workflow run hands every agent call it makes.
fn workflow_parent(root: &std::path::Path, workspace: &std::path::Path) -> ToolContext {
    ToolContext {
        write_roots: vec![workspace.to_path_buf()],
        denied_directory_names: vec!["secret".into()],
        run_store: Some(Default::default()),
        ..parent(root, &[])
    }
}

fn workflow_request(workspace: &std::path::Path) -> archon_tools::subagent_request::SubagentRequest {
    let mut request = request(workspace, None, vec![]);
    request.write_roots = vec![workspace.display().to_string()];
    request
}

#[tokio::test]
async fn after_a_restart_the_same_workflow_call_runs_as_a_new_agent_with_its_limits() {
    let (_t, root) = real_temp();
    let workspace = dir(&root, "workspace");
    let outside = root.join("outside.txt");
    let store = store(&root);
    let id = "run-1-stage-implement-attempt-1";
    let before = Host::new(&root, "workflow-restart", vec![STOP]);
    before
        .spawn(id, workflow_request(&workspace), workflow_parent(&root, &workspace))
        .await
        .unwrap();
    history(&store, id);
    // A new process: a new manager, and the transcript still on disk.
    let after = Host::new(
        &root,
        "workflow-restart",
        vec![write(&outside, "escaped"), write(&workspace.join("ok"), "in"), STOP],
    );
    after
        .spawn(id, workflow_request(&workspace), workflow_parent(&root, &workspace))
        .await
        .expect("a workflow call after a restart must start, not refuse");
    assert!(after.outcome(0).is_error, "the workflow's write roots were lost");
    assert!(!outside.exists());
    assert!(!after.outcome(1).is_error, "{:?}", after.outcome(1));
}

#[tokio::test]
async fn a_repair_with_no_stored_context_continues_under_the_workflow_definition() {
    let (_t, root) = real_temp();
    let workspace = dir(&root, "workspace");
    let outside = root.join("outside.txt");
    let id = "run-1-0-coder-generation";
    let session = CompletedHistory::default();
    let first = Host::new(&root, "workflow-repair", vec![STOP]);
    scope(
        SubagentSession {
            agent_id: id.into(),
            history: session.clone(),
            continuing: false,
        },
        first.spawn(id, workflow_request(&workspace), workflow_parent(&root, &workspace)),
    )
    .await
    .unwrap();
    // A manager with no entry for `id`, as after its context was collected.
    let host = Host::new(&root, "workflow-repair", vec![write(&outside, "escaped"), STOP]);
    let result = scope(
        SubagentSession {
            agent_id: id.into(),
            history: session,
            continuing: true,
        },
        host.spawn(id, workflow_request(&workspace), workflow_parent(&root, &workspace)),
    )
    .await;
    assert!(result.is_ok(), "the repair stopped the run: {result:?}");
    assert!(host.outcome(0).is_error, "the workflow's write roots were lost");
    assert!(!outside.exists());
}

#[tokio::test]
async fn a_repair_after_its_clean_worktree_was_removed_continues_under_the_workflow() {
    use archon_core::agent::AgentConfig;
    use archon_tools::isolation::AutoIsolation;
    let (_t, root) = real_temp();
    let repo = checkout(&root);
    let id = "run-1-0-coder-worktree";
    let session = CompletedHistory::default();
    let host = Host::with_config(
        &root,
        "workflow-worktree",
        vec![STOP, STOP],
        AgentConfig {
            subagent_auto_isolation: AutoIsolation::Always,
            ..Default::default()
        },
    );
    let call = |continuing| {
        scope(
            SubagentSession {
                agent_id: id.into(),
                history: session.clone(),
                continuing,
            },
            host.spawn(id, workflow_request(&repo), workflow_parent(&root, &repo)),
        )
    };
    call(false).await.unwrap();
    // The first call made no change, so its completion removes the worktree.
    host.executor
        .on_visible_complete(id.into(), Ok("unparsable".into()), false)
        .await;
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
    let result = call(true).await;
    assert!(result.is_ok(), "the repair stopped the run: {result:?}");
}
