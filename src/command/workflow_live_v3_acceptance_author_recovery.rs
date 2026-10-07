//! A recovery read may publish or roll back bytes: fence its entire read.
use super::{ACCEPTANCE_LOCK_FILE, AcceptanceContract, StageContext, WorkflowResult};
use std::path::{Path, PathBuf};

pub(super) enum Candidate {
    Frozen,
    Named(Option<PathBuf>),
    Invalid(String),
}
pub(super) fn candidate(context: &StageContext, pin: &Path) -> WorkflowResult<Candidate> {
    archon_workflow::stage_write::with_write(|| {
        #[cfg(test)]
        super::recovery_tests::entry_hook();
        // Take the run lock before waiting for publication and keep it until
        // settlement and this consistent read finish. Task-local ownership
        // alone does not fence the recovery mutations that a read invokes.
        let _read =
            crate::command::workflow_task_set::ChainRead::workflow_at(pin, &context.task_root)?;
        if context.task_root.join(ACCEPTANCE_LOCK_FILE).exists() || pin.exists() {
            return Ok(Candidate::Frozen);
        }
        Ok(match std::fs::read(context.contract_path()) {
            Ok(bytes) => match serde_json::from_slice::<AcceptanceContract>(&bytes) {
                Ok(candidate) => {
                    Candidate::Named(Some(super::super::drift::prd_path(context, &candidate)))
                }
                Err(error) => Candidate::Invalid(format!(
                    "the unfrozen candidate {} is not an acceptance contract ({error}), so the PRD it answers to is not recorded",
                    context.contract_path().display()
                )),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Candidate::Named(None),
            Err(error) => Candidate::Invalid(format!(
                "the unfrozen candidate {} cannot be read: {error}",
                context.contract_path().display()
            )),
        })
    })
}
