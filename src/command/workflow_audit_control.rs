//! CLI inspection is read-only. Mutation requires the interactive host channel.
use std::path::Path;
use crate::cli_args::workflow_audit::AuditAction;
use archon_workflow::{WorkflowStore, WorkflowV2ResultStore};

pub(crate) async fn handle_cli(project: &Path, action: &AuditAction) -> anyhow::Result<()> {
    let run_id = action.run_id();
    if run_id.is_empty() || run_id.contains(['/', '\\']) || run_id == "." || run_id == ".." {
        anyhow::bail!("invalid audit run ID");
    }
    let store = WorkflowStore::project(project);
    let v2 = WorkflowV2ResultStore::new(store.run_dir(run_id).join("v2"));
    let state = archon_workflow::repository_audit::reuse::load_state(&v2)?
        .ok_or_else(|| anyhow::anyhow!("run has no repository audit"))?;
    match action {
        AuditAction::Status { .. } => println!("{}", serde_json::to_string_pretty(&state)?),
        _ => anyhow::bail!("audit mutation requires confirmation through the interactive host operator channel; no mutation applied"),
    }
    Ok(())
}
