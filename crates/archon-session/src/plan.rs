pub use crate::plan_models::{
    PlanApproval, PlanApprovalDecision, PlanApprovalRecord, PlanApprovalSource, PlanDocument,
    PlanReconciliationStatus, PlanStatus, PlanStep, PlanStepDependency, PlanStepReconciliation,
    PlanStepStatus,
};
pub use crate::plan_store::PlanStore;

/// Build a plan context string suitable for injection into compaction summaries.
/// Returns None if no active plan exists.
pub fn plan_context_for_compaction(store: &PlanStore, session_id: &str) -> Option<String> {
    match store.load_latest_plan(session_id) {
        Ok(Some(plan)) if matches!(plan.status, PlanStatus::Executing | PlanStatus::Draft) => {
            Some(format!("\n\n---\n[Active Plan]\n{}", plan.to_context_string()))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use cozo::DbInstance;

    use super::*;

    fn test_db() -> DbInstance {
        DbInstance::new("mem", "", "").expect("in-memory db")
    }

    #[test]
    fn legacy_plan_json_loads_with_safe_defaults() {
        let json = r#"{"id":"p","title":"Legacy","steps":[],"risks":[],"questions":[],"status":"active"}"#;
        let plan = PlanDocument::from_json(json).unwrap();
        assert_eq!(plan.status, PlanStatus::Executing);
        assert!(plan.approval.is_none());
        assert!(plan.reconciliation.is_empty());
    }

    #[test]
    fn approval_events_roundtrip_in_durable_ledger() {
        let db = test_db();
        let store = PlanStore::new(&db).expect("init");
        let record = PlanApprovalRecord {
            plan_id: "plan-approval".into(),
            session_id: "session-approval".into(),
            approval: PlanApproval {
                decision: PlanApprovalDecision::ApproveAcceptEdits,
                source: PlanApprovalSource::NonInteractive,
                decided_at: "2026-08-14T00:00:00Z".into(),
                user_edited: true,
            },
        };
        store.record_approval_event(&record).expect("record");
        let duplicate = store.record_approval_event(&record);
        assert!(duplicate.is_err(), "approval ledger must not overwrite events");
        assert_eq!(
            store
                .load_approval_events("session-approval", "plan-approval")
                .expect("load"),
            vec![record]
        );
    }

    #[test]
    fn plan_roundtrip() {
        let db = test_db();
        let store = PlanStore::new(&db).expect("init");
        let mut plan = PlanDocument::new("plan-1", "Test Plan");
        plan.steps.push(step(1, "First step", PlanStepStatus::Pending));
        plan.steps.push(step(2, "Second step", PlanStepStatus::Pending));
        plan.risks.push("Might break things".to_string());
        store.save_plan("sess1", &plan).expect("save");
        let loaded = store
            .load_plan("sess1", "plan-1")
            .expect("load")
            .expect("found");
        assert_eq!(loaded.title, "Test Plan");
        assert_eq!(loaded.steps.len(), 2);
        assert_eq!(loaded.steps[0].description, "First step");
        assert_eq!(loaded.risks.len(), 1);
    }

    #[test]
    fn update_step_status() {
        let db = test_db();
        let store = PlanStore::new(&db).expect("init");
        let mut plan = PlanDocument::new("plan-2", "Step Test");
        plan.steps.push(step(1, "Do something", PlanStepStatus::Pending));
        store.save_plan("sess1", &plan).expect("save");
        store
            .update_step_status("sess1", "plan-2", 1, PlanStepStatus::Complete)
            .expect("update");
        let loaded = store
            .load_plan("sess1", "plan-2")
            .expect("load")
            .expect("found");
        assert_eq!(loaded.steps[0].status, PlanStepStatus::Complete);
    }

    #[test]
    fn completion_percentage() {
        let mut plan = PlanDocument::new("p", "t");
        assert_eq!(plan.completion_pct(), 0.0);
        plan.steps.push(step(1, "a", PlanStepStatus::Complete));
        plan.steps.push(step(2, "b", PlanStepStatus::Pending));
        assert!((plan.completion_pct() - 50.0).abs() < 0.1);
        plan.steps[1].status = PlanStepStatus::Skipped;
        assert!((plan.completion_pct() - 100.0).abs() < 0.1);
    }

    #[test]
    fn to_context_string_format() {
        let mut plan = PlanDocument::new("p", "My Plan");
        plan.status = PlanStatus::Executing;
        plan.steps.push(step(1, "Step one", PlanStepStatus::Complete));
        plan.steps.push(step(2, "Step two", PlanStepStatus::Pending));
        plan.steps[0].affected_files.push("a.rs".into());
        let context = plan.to_context_string();
        assert!(context.contains("My Plan"));
        assert!(context.contains("[x] 1. Step one"));
        assert!(context.contains("[ ] 2. Step two"));
        assert!(context.contains("a.rs"));
    }

    #[test]
    fn load_nonexistent_returns_none() {
        let db = test_db();
        let store = PlanStore::new(&db).expect("init");
        assert!(store.load_plan("sess1", "nope").expect("load").is_none());
    }

    fn step(number: u32, description: &str, status: PlanStepStatus) -> PlanStep {
        PlanStep {
            number,
            description: description.into(),
            affected_files: Vec::new(),
            status,
            blocked_by: Vec::new(),
            required_evidence: Vec::new(),
            task_id: None,
        }
    }
}
