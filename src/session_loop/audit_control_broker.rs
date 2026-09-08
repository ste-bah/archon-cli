//! External callers may request a control; approval remains in human input.
use crate::cli_args::workflow_audit::AuditAction;
use std::path::Path;

pub(super) struct RequestBroker;
impl RequestBroker {
    pub(super) async fn start(_project: &Path) -> anyhow::Result<Self> {
        anyhow::bail!("operator request broker not connected")
    }
    pub(super) async fn receive(&mut self) -> Option<AuditAction> { None }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn repository_audit_cli_request_reaches_host_without_mutating_state() {
        use archon_workflow::repository_audit::{budget::{AuditPolicy,Limit},runtime::AuditRuntime};
        let temp=tempfile::tempdir().unwrap();
        let store=archon_workflow::WorkflowStore::project(temp.path());
        let run=store.create_run(archon_workflow::WorkflowSpec{schema:archon_workflow::spec::WORKFLOW_SCHEMA.into(),name:"control".into(),task:"audit".into(),target_repository_root:None,max_agents:1,max_parallelism:1,stages:vec![],permissions:Default::default(),learning_hooks:vec![]}).unwrap();
        let audit=AuditRuntime::initialize(store,run.id.clone(),AuditPolicy{attempt_timeout_secs:Limit::Finite(10),total_time_secs:Limit::Finite(20),unexpected_change_refreshes:Limit::Finite(3)}).unwrap();
        let mut broker=RequestBroker::start(temp.path()).await.unwrap();
        let action=AuditAction::ExtendBudget{run_id:run.id,extra_refreshes:Some(2),extra_seconds:None,reason:"more source".into()};
        crate::command::workflow_audit_control::handle_cli(temp.path(),&action).await.unwrap();
        let queued=tokio::time::timeout(std::time::Duration::from_secs(2),broker.receive()).await.unwrap().unwrap();
        assert_eq!(queued.run_id(),audit.run_id);
        assert_eq!(audit.state().unwrap().budget.policy.unexpected_change_refreshes,Limit::Finite(3));
        assert!(audit.state().unwrap().operator_controls.is_empty());
    }
}
