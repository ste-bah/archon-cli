//! Pending operator requests are private to the interactive input host.
use crate::cli_args::workflow_audit::AuditAction;
use std::path::{Path, PathBuf};

pub(crate) struct PendingControl {
    project: PathBuf,
    action: AuditAction,
    generation: u64,
    id: String,
    prior_policy: archon_workflow::repository_audit::budget::AuditPolicy,
}
impl PendingControl {
    fn prepare(project: &Path, action: AuditAction) -> anyhow::Result<Self> {
        let store = archon_workflow::WorkflowStore::project(project);
        crate::command::workflow_audit_control::validate_run_id(action.run_id())?;
        let generation = store.load_state(action.run_id())?.generation;
        let state = crate::command::workflow_audit_control::read_state(&store, action.run_id())?;
        mutation::updated_policy(&action, &state.budget.policy)?;
        Ok(Self { project: project.into(), action, generation, id: uuid::Uuid::new_v4().to_string(), prior_policy: state.budget.policy })
    }
    fn confirm(self, confirmation: &str) -> anyhow::Result<()> {
        if confirmation != format!("/workflow audit confirm {}", self.id) {
            anyhow::bail!("confirmation does not match the pending audit request");
        }
        let store = archon_workflow::WorkflowStore::project(&self.project);
        store.with_run_lock(self.action.run_id(), |store| {
            use archon_workflow::{WorkflowError, repository_audit::runtime::STATE_PATH};
            let run = store.load_state(self.action.run_id())?;
            if run.generation != self.generation || matches!(run.status, archon_workflow::RunStatus::Completed | archon_workflow::RunStatus::Cancelled) {
                return Err(WorkflowError::PolicyDenied("audit confirmation has stale generation or terminal run".into()));
            }
            let mut state = crate::command::workflow_audit_control::read_state(store, self.action.run_id())?;
            if state.generation != self.generation || state.budget.policy != self.prior_policy {
                return Err(WorkflowError::PolicyDenied("audit policy changed since confirmation was requested".into()));
            }
            let policy = mutation::updated_policy(&self.action, &state.budget.policy)?;
            let record = serde_json::json!({"action_id":self.id,"action":self.action,
                "generation":self.generation,"channel":"interactive-human-input",
                "timestamp":chrono::Utc::now().to_rfc3339(),"prior_policy":state.budget.policy,
                "new_policy":policy,"spent_ms":state.budget.spent_ms,
                "unexpected_refreshes":state.budget.unexpected_refreshes});
            state.budget.policy = policy;
            state.final_receipt = None;
            state.operator_controls.push(record);
            store.write_run_json(self.action.run_id(), STATE_PATH, &state)
        })?;
        Ok(())
    }
}

#[derive(Default)]
pub(super) struct OperatorInbox {
    pending: Option<PendingControl>,
}
impl OperatorInbox {
    pub(super) fn handle_input(&mut self, project: &Path, input: &str) -> Option<anyhow::Result<String>> {
        let input = super::slash_input(input)?;
        if !input.starts_with("/workflow audit ") { return None; }
        Some(self.dispatch(project, &input))
    }

    fn dispatch(&mut self, project: &Path, input: &str) -> anyhow::Result<String> {
        if input.starts_with("/workflow audit confirm ") {
            let pending = self.pending.take().ok_or_else(|| anyhow::anyhow!("no pending audit confirmation"))?;
            pending.confirm(input)?;
            return Ok("Audit control recorded. Consumption retained; no workflow was launched or resumed.\n".into());
        }
        if input == "/workflow audit cancel" {
            self.pending = None;
            return Ok("Pending audit control discarded; run unchanged.\n".into());
        }
        use clap::Parser;
        let parsed = crate::command::parser::CommandParser::parse(input)?;
        let args = [vec!["archon".to_string(), "workflow".to_string()], parsed.raw_args].concat();
        let cli = crate::cli_args::Cli::try_parse_from(args)?;
        let Some(crate::cli_args::Commands::Workflow { action: crate::cli_args::WorkflowAction::Audit { action } }) = cli.command else {
            anyhow::bail!("invalid audit control command");
        };
        if let AuditAction::Status { run_id } = &action {
            let store = archon_workflow::WorkflowStore::project(project);
            return Ok(serde_json::to_string_pretty(&crate::command::workflow_audit_control::read_state(&store, run_id)?)?);
        }
        let pending = PendingControl::prepare(project, action)?;
        let preview = format!("Audit control request (not applied):\n{}\nGeneration: {}\nPrior policy: {}\nConfirm exactly, or use /workflow audit cancel:\n/workflow audit confirm {}\n",
            serde_json::to_string_pretty(&pending.action)?, pending.generation,
            serde_json::to_string(&pending.prior_policy)?, pending.id);
        self.pending = Some(pending);
        Ok(preview)
    }
}

#[path = "audit_control_mutation.rs"]
mod mutation;

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
    #[test]
    fn repository_audit_human_input_dispatch_requires_exact_confirmation() {
        let (temp,audit)=fixture();
        let mut inbox=OperatorInbox::default();
        let request=format!("/workflow audit extend-budget {} --extra-refreshes 2 --reason \"larger repository\"",audit.run_id);
        let preview=inbox.handle_input(temp.path(),&request).expect("operator input must be intercepted").unwrap();
        assert_eq!(audit.state().unwrap().budget.policy.unexpected_change_refreshes,Limit::Finite(3));
        let confirmation=preview.lines().find(|line| line.starts_with("/workflow audit confirm ")).unwrap().to_string();
        inbox.handle_input(temp.path(),&confirmation).unwrap().unwrap();
        assert_eq!(audit.state().unwrap().budget.policy.unexpected_change_refreshes,Limit::Finite(5));
        assert!(inbox.handle_input(temp.path(),&confirmation).unwrap().is_err(),"confirmation cannot replay");
        assert_eq!(audit.state().unwrap().operator_controls.len(),1);
    }

}
