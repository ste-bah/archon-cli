use archon_session::plan::{PlanStatus, PlanStore, reconcile_durable_plan, reconciliation_summary};

/// Return a durable reconciliation block for a completion claim, if an active
/// approved plan has omitted, deviated, or unplanned work.
pub fn completion_block_for_session(session_id: &str) -> Option<String> {
    completion_block_for_session_at_path(&archon_session::storage::default_db_path(), session_id)
}

/// Read reconciliation from the session database selected by the production
/// runtime. Callers with configured session storage must use this rather than
/// falling back to the process default.
pub fn completion_block_for_session_at_path(
    database: &std::path::Path,
    session_id: &str,
) -> Option<String> {
    let store = archon_session::storage::SessionStore::open(database).ok()?;
    let plan_store = PlanStore::new(store.db()).ok()?;
    completion_block_for_plan_store(&plan_store, session_id)
}

/// Reconcile the active durable plan and return a compact completion block.
pub fn completion_block_for_plan_store(plan_store: &PlanStore, session_id: &str) -> Option<String> {
    let plan = plan_store.load_latest_plan(session_id).ok()??;
    if !matches!(plan.status, PlanStatus::Approved | PlanStatus::Executing) {
        return None;
    }
    let tasks = plan_store.load_plan_tasks(session_id).ok()?;
    let reconciliation = reconcile_durable_plan(&plan, &tasks);
    let summary = reconciliation_summary(&reconciliation)?;
    Some(format!(
        "Plan completion is blocked. {summary} Complete every approved plan task with its required durable evidence and remove or explicitly plan extra file changes."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use archon_session::plan::{PlanDocument, PlanStep, PlanStepStatus};

    #[test]
    fn completion_block_reads_the_explicit_runtime_session_database() {
        let temp = tempfile::tempdir().unwrap();
        let database = temp.path().join("runtime-session.db");
        let session_id = "runtime-session";
        let session_store = archon_session::storage::SessionStore::open(&database).unwrap();
        let plan_store = PlanStore::new(session_store.db()).unwrap();
        let mut plan = PlanDocument::new("runtime-plan", "Runtime plan");
        plan.session_id = Some(session_id.into());
        plan.status = PlanStatus::Executing;
        plan.steps = vec![PlanStep {
            number: 1,
            description: "finish the approved work".into(),
            affected_files: Vec::new(),
            status: PlanStepStatus::Pending,
            blocked_by: Vec::new(),
            required_evidence: Vec::new(),
            task_id: None,
        }];
        plan_store.save_plan(session_id, &plan).unwrap();

        let block = completion_block_for_session_at_path(&database, session_id);

        assert!(block.is_some());
    }
}
