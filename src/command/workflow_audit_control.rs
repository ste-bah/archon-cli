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

#[cfg(test)]
mod tests {
    use super::*;
    use archon_workflow::repository_audit::{budget::{AuditPolicy, Limit}, runtime::AuditRuntime};

    #[tokio::test]
    async fn repository_audit_cli_mutation_without_host_confirmation_changes_nothing() {
        let temp = tempfile::tempdir().unwrap();
        let store = WorkflowStore::project(temp.path());
        let run = store.create_run(archon_workflow::WorkflowSpec {
            schema:archon_workflow::spec::WORKFLOW_SCHEMA.into(), name:"operator".into(),task:"audit".into(),
            target_repository_root:None,max_agents:1,max_parallelism:1,stages:vec![],permissions:Default::default(),learning_hooks:vec![],
        }).unwrap();
        let audit = AuditRuntime::initialize(store.clone(),run.id.clone(),AuditPolicy{
            attempt_timeout_secs:Limit::Finite(10),total_time_secs:Limit::Finite(20),unexpected_change_refreshes:Limit::Finite(3),
        }).unwrap();
        let before = std::fs::read(store.run_dir(&run.id).join(archon_workflow::repository_audit::runtime::STATE_PATH)).unwrap();
        let action = AuditAction::ExtendBudget{run_id:run.id.clone(),extra_refreshes:Some(2),extra_seconds:None,reason:"more time".into()};
        assert!(handle_cli(temp.path(),&action).await.is_err());
        assert_eq!(before,std::fs::read(store.run_dir(&run.id).join(archon_workflow::repository_audit::runtime::STATE_PATH)).unwrap());
        assert_eq!(audit.state().unwrap().budget.policy.unexpected_change_refreshes,Limit::Finite(3));
    }
}
