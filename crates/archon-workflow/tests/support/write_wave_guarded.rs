//! Batch G2: the write-wave harness's guarded shell -- a branch's command
//! run through the real Bash tool under the guard its host stamps build.
use archon_workflow::agent_dispatch_port;
use serde_json::json;
use std::path::Path;

/// Batch G2: an edit's content prefix that runs the rest through the real
/// Bash tool, under the guard the host's stamps on the branch's input build
/// -- the live dispatch's own shell, its OS write boundary included.
pub const BASH: &str = "\u{0}bash:";
/// Batch G2: an edit path prefix naming the branch's own copy of a declared
/// project artifact (`@copy:<project-relative path>`), as the host stamped it.
pub const COPY: &str = "@copy:";
/// Batch G2: an edit's content prefix that writes the rest through the file
/// tool's guard (`Write`), as an agent's file tool would; a refusal is kept.
pub const WRITE: &str = "\u{0}write:";

/// The guard the live dispatch builds for a write branch: from its input
/// stamps, under the run-store scope of the run at `run_root`.
fn branch_guard(
    root: &Path,
    run_root: &Path,
    input: &serde_json::Value,
) -> archon_tools::workflow_read_guard::WorkflowReadGuard {
    use archon_tools::workflow_read_guard as guard;
    let declared = guard::DeclaredTargetScope::new(
        &agent_dispatch_port::declared_targets(input),
        root.to_str(),
    )
    .in_isolated_worktree(agent_dispatch_port::isolated_worktree(input));
    let declared = match agent_dispatch_port::write_boundary(input) {
        Some((sealed, writable)) => {
            declared.with_write_boundary(guard::HostWriteBoundary::new(&sealed, &writable))
        }
        None => declared,
    };
    let store = run_root.parent().and_then(Path::to_str);
    let run_store = guard::RunStoreScope::new(run_root.to_str(), store, root.to_str());
    guard::WorkflowReadGuard::from_settings(&guard::WorkflowReadGuardSettings::default())
        .with_declared_targets(declared)
        .with_run_store(run_store)
}

/// Write `content` at `target` as an agent's `Write` tool would: `Some`
/// refusal when the guard refuses it (nothing written).
pub(super) fn guarded_write(
    root: &Path,
    run_root: &Path,
    input: &serde_json::Value,
    target: &Path,
    content: &str,
) -> Option<String> {
    let refused = branch_guard(root, run_root, input)
        .before_tool("Write", &json!({"file_path": target, "content": content}));
    if refused.is_none() {
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, content).unwrap();
    }
    refused
}

/// Run `command` in `root` through the Bash tool as the live dispatch does:
/// the guard built from the branch's input stamps, under the run-store
/// scope of the run at `run_root` (the live dispatch's `scope_run_store`).
pub(super) async fn guarded_bash(
    root: &Path,
    run_root: &Path,
    input: &serde_json::Value,
    command: &str,
) -> String {
    let guard = branch_guard(root, run_root, input);
    let ctx = archon_tools::tool::ToolContext {
        working_dir: root.to_path_buf(),
        session_id: "write-wave-fixture".into(),
        workflow_read_guard: Some(std::sync::Arc::new(guard)),
        ..archon_tools::tool::ToolContext::default()
    };
    let result = archon_tools::tool::Tool::execute(
        &archon_tools::bash::BashTool::default(),
        json!({"command": command}),
        &ctx,
    )
    .await;
    result.content
}

/// The artifact context as the live dispatch builds it: what the prompt's
/// declared-artifact section and the completion check read.
pub(super) fn live_artifact_context(
    request: &mut archon_workflow::WorkflowV2AgentRequest,
    store: Option<&archon_workflow::WorkflowV2ResultStore>,
) {
    if let Some(store) = store {
        let mut context = archon_workflow::project_artifact_context_from_v2_root(store.root());
        context.repository_root = request.repository_root.clone();
        context.add_artifact_requirements(&request.input);
        request.project_artifacts = context;
    }
}

/// Where an edit of `path` writes: the branch's copy of a `@copy:` path, as
/// the host stamped it, or the worktree path.
pub(super) fn edit_target(
    root: &Path,
    path: &str,
    input: &serde_json::Value,
    store: Option<&archon_workflow::WorkflowV2ResultStore>,
) -> std::path::PathBuf {
    let Some(rel) = path.strip_prefix(COPY) else {
        return root.join(path);
    };
    let project = archon_workflow::project_artifact_context_from_v2_root(store.unwrap().root())
        .project_root
        .unwrap();
    let wanted = Path::new(&project).join(rel).display().to_string();
    let copies = agent_dispatch_port::artifact_copies(input);
    let copy = copies.iter().find(|(path, _)| *path == wanted);
    std::path::PathBuf::from(
        &copy
            .unwrap_or_else(|| panic!("no copy of {wanted}: {copies:?}"))
            .1,
    )
}
