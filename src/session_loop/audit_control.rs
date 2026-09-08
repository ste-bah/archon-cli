//! Pending operator requests are private to the interactive input host.
use crate::cli_args::workflow_audit::AuditAction;
use std::path::{Path, PathBuf};

pub(crate) struct PendingControl {
    project: PathBuf,
    action: AuditAction,
    generation: u64,
    id: String,
}
impl PendingControl {
    fn prepare(project: &Path, action: AuditAction) -> anyhow::Result<Self> {
        let store = archon_workflow::WorkflowStore::project(project);
        let generation = store.load_state(action.run_id())?.generation;
        Ok(Self { project: project.into(), action, generation, id: uuid::Uuid::new_v4().to_string() })
    }
    fn confirm(self, confirmation: &str) -> anyhow::Result<()> {
        let _ = (&self.project, &self.action, self.generation, confirmation);
        anyhow::bail!("operator confirmation is not connected")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archon_workflow::repository_audit::{budget::{AuditPolicy, Limit}, runtime::AuditRuntime};
    fn fixture() -> (tempfile::TempDir, AuditRuntime) {
        let temp = tempfile::tempdir().unwrap();
        let store = archon_workflow::WorkflowStore::project(temp.path());
        let run = store.create_run(archon_workflow::WorkflowSpec {
            schema:archon_workflow::spec::WORKFLOW_SCHEMA.into(), name:"operator".into(),task:"audit".into(),
            target_repository_root:None,max_agents:1,max_parallelism:1,stages:vec![],permissions:Default::default(),learning_hooks:vec![],
        }).unwrap();
        let audit = AuditRuntime::initialize(store,run.id,AuditPolicy{
            attempt_timeout_secs:Limit::Finite(10),total_time_secs:Limit::Finite(20),unexpected_change_refreshes:Limit::Finite(3),
        }).unwrap();
        audit.update(|s| { s.budget.spent_ms=500; s.budget.unexpected_refreshes=2; Ok(()) }).unwrap();
        (temp,audit)
    }
    #[test]
    fn repository_audit_confirmed_extension_preserves_consumption() {
        let (temp,audit)=fixture();
        let action=AuditAction::ExtendBudget{run_id:audit.run_id.clone(),extra_refreshes:Some(2),extra_seconds:Some(40),reason:"operator allowance".into()};
        let pending=PendingControl::prepare(temp.path(),action).unwrap();
        assert_eq!(audit.state().unwrap().budget.policy.total_time_secs,Limit::Finite(20));
        let confirmation=format!("/workflow audit confirm {}",pending.id);
        pending.confirm(&confirmation).unwrap();
        let state=audit.state().unwrap();
        assert_eq!(state.budget.policy.total_time_secs,Limit::Finite(60));
        assert_eq!(state.budget.policy.unexpected_change_refreshes,Limit::Finite(5));
        assert_eq!(state.budget.spent_ms,500);
        assert_eq!(state.budget.unexpected_refreshes,2);
    }
    #[test]
    fn repository_audit_wrong_confirmation_changes_nothing() {
        let (temp,audit)=fixture();
        let action=AuditAction::ExtendBudget{run_id:audit.run_id.clone(),extra_refreshes:Some(2),extra_seconds:None,reason:"operator allowance".into()};
        let pending=PendingControl::prepare(temp.path(),action).unwrap();
        assert!(pending.confirm("yes").is_err());
        assert_eq!(audit.state().unwrap().budget.policy.unexpected_change_refreshes,Limit::Finite(3));
    }
    #[test]
    fn repository_audit_generation_change_invalidates_confirmation() {
        let (temp,audit)=fixture();
        let action=AuditAction::ExtendBudget{run_id:audit.run_id.clone(),extra_refreshes:Some(2),extra_seconds:None,reason:"operator allowance".into()};
        let pending=PendingControl::prepare(temp.path(),action).unwrap();
        let mut run=audit.store.load_state(&audit.run_id).unwrap();
        run.generation+=1;
        audit.store.save_state(&run).unwrap();
        let confirmation=format!("/workflow audit confirm {}",pending.id);
        assert!(pending.confirm(&confirmation).is_err());
        assert_eq!(audit.state().unwrap().budget.policy.unexpected_change_refreshes,Limit::Finite(3));
    }
}
