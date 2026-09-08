//! Last deterministic gate before terminal projection; never invoke an observer here.
use super::WorkflowV2ScriptSummary;
use archon_workflow::{WorkflowError, WorkflowResult, WorkflowStore, WorkflowV2Status};
use archon_workflow::repository_audit::runtime::{AuditState, STATE_PATH};

pub(super) fn gate(store: &WorkflowStore, run_id: &str, summary: &WorkflowV2ScriptSummary) -> WorkflowResult<WorkflowV2ScriptSummary> {
    let mut summary = summary.clone();
    if !matches!(summary.status, WorkflowV2Status::Accepted | WorkflowV2Status::Noop) {
        return Ok(summary);
    }
    let path = store.run_dir(run_id).join(STATE_PATH);
    // Old records predate mandatory registration. New runs write the marker
    // separately so a missing state file cannot quietly become a legacy run.
    let required = store.run_dir(run_id).join("v2/repository-audit/required.json").exists();
    if !required && !path.exists() { return Ok(summary); }
    let verdict = (|| {
        let state: AuditState = serde_json::from_slice(&std::fs::read(&path)
            .map_err(|e| WorkflowError::Io { path: path.clone(), source: e })?)?;
        if state.schema_version != 1 || state.generation != store.load_state(run_id)?.generation {
            return Err(WorkflowError::StateCorrupt("repository audit identity changed before finalization".into()));
        }
        if state.last_error.is_some() || state.budget.active.is_some() {
            return Err(WorkflowError::StageFailed("repository audit assessment unavailable or still active".into()));
        }
        let snapshot = state.snapshot.as_ref().ok_or_else(|| WorkflowError::StateCorrupt("repository audit has no final snapshot".into()))?;
        let unresolved = state.ledger.unresolved(&snapshot.identity)?;
        if !unresolved.is_empty() {
            return Err(WorkflowError::StageFailed(format!("repository audit unresolved paths: {}", unresolved.join(", "))));
        }
        Ok(())
    })();
    if let Err(error) = verdict {
        summary.status = WorkflowV2Status::Failed;
        summary.failed_call = Some("repository-audit-final".into());
        summary.failed_result_path = Some(path.display().to_string());
        summary.next_action = Some(error.to_string());
    }
    Ok(summary)
}
