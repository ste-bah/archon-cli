//! REM-14: the mandatory reviews of the escalation harness, answered as a
//! reviewer that found nothing answers: every map branch the host dispatched
//! reviewed its task and reported no finding, the reduce added none. The
//! host attaches its findings to the record exactly as live
//! (`normalize_and_attach_review_findings`), so the prelude reads the
//! attachment it reads live.
use archon_workflow::v2::call_data::dispatched_items;
use archon_workflow::v2::script::normalize_and_attach_review_findings;
use archon_workflow::*;
use serde_json::{Value, json};

use super::{Answer, Host};

impl Host {
    pub(super) fn review(&self, execution: WorkflowV2CallExecution) -> Value {
        let branch = |item_id: &str, tasks: &[String]| {
            json!({"item_id": item_id, "id": item_id, "status": "accepted",
                "canonical_task_ids": tasks,
                "result": {"status": "accepted", "summary": "reviewed: no findings",
                    "evidence": [{"kind": "review", "summary": "reviewed"}], "data": {"findings": []}}})
        };
        let outcomes: Vec<Value> = dispatched_items(&execution)
            .iter()
            .map(|item| branch(&item.item_id, &item.canonical_task_ids))
            .collect();
        let mut result = WorkflowV2Result::accepted("reviewed: no findings");
        result.evidence.push(WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Review,
            "reviewed",
        ));
        result.data = if outcomes.is_empty() {
            json!({"findings": []})
        } else {
            json!({"outcomes": outcomes})
        };
        let result = normalize_and_attach_review_findings(
            &execution,
            result,
            &self.store,
            self.f.universe.as_ref(),
        )
        .unwrap();
        let record = self.save(&execution, result);
        self.note(&execution, Answer::Ran);
        self.view(&record)
    }
}

/// REM-14: a write's remediation contract, or `null` for a task write
/// outside remediation (a host completion unit's).
pub(super) fn remediation_contract_of(execution: &WorkflowV2CallExecution) -> Value {
    execution
        .call
        .options
        .extra
        .get("remediationContract")
        .cloned()
        .unwrap_or_default()
}
