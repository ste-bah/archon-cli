use std::collections::BTreeSet;

use archon_completion::RequiredEvidence;
use archon_session::plan::{PlanStatus, reconciliation_summary};

use super::*;

#[derive(Debug, Clone, Default)]
pub struct PlanExecutionEvidence {
    pub touched_files: BTreeSet<String>,
    pub completion: Vec<RequiredEvidence>,
}

impl Agent {
    pub(super) fn record_plan_file_mutation(&mut self, file_path: &str) -> Result<(), String> {
        self.plan_execution_evidence
            .touched_files
            .insert(file_path.to_string());
        let Some(store) = self.plan_store.as_ref() else {
            return Ok(());
        };
        let Some(plan) = store
            .load_latest_plan(&self.config.session_id)
            .map_err(|error| format!("failed to load active plan: {error}"))?
        else {
            return Ok(());
        };
        if matches!(plan.status, PlanStatus::Approved | PlanStatus::Executing) {
            store
                .record_plan_file_mutation(&self.config.session_id, &plan.id, file_path)
                .map_err(|error| format!("failed to persist plan file mutation: {error}"))?;
        }
        Ok(())
    }

    pub(super) fn record_plan_observation_failure(&mut self, failure: &str) -> Result<(), String> {
        let result = (|| {
            let Some(store) = self.plan_store.as_ref() else {
                return Ok(());
            };
            let Some(plan) = store
                .load_latest_plan(&self.config.session_id)
                .map_err(|error| format!("failed to load active plan: {error}"))?
            else {
                return Ok(());
            };
            if matches!(plan.status, PlanStatus::Approved | PlanStatus::Executing) {
                store
                    .record_plan_observation_failure(&self.config.session_id, &plan.id, failure)
                    .map_err(|error| format!("failed to persist observation failure: {error}"))?;
            }
            Ok(())
        })();
        if result.is_ok() {
            self.observation_failure_blocker = None;
        } else {
            self.observation_failure_blocker = Some(failure.to_string());
        }
        result
    }

    pub(super) fn retry_pending_observation_failure(&mut self) -> Result<(), String> {
        let Some(failure) = self.observation_failure_blocker.clone() else {
            return Ok(());
        };
        let store = self
            .plan_store
            .as_ref()
            .ok_or_else(|| "pending observation failure has no durable plan store".to_string())?;
        let plan = store
            .load_latest_plan(&self.config.session_id)
            .map_err(|error| format!("failed to load active plan for observation retry: {error}"))?
            .filter(|plan| matches!(plan.status, PlanStatus::Approved | PlanStatus::Executing))
            .ok_or_else(|| "pending observation failure has no active durable plan".to_string())?;
        store
            .record_plan_observation_failure(&self.config.session_id, &plan.id, &failure)
            .map_err(|error| format!("failed to retry observation failure persistence: {error}"))?;
        self.observation_failure_blocker = None;
        Ok(())
    }

    pub(super) fn record_plan_completion_evidence(&mut self, input: &serde_json::Value) {
        let Some(store) = self.plan_store.as_ref() else {
            return;
        };
        let Some(task_id) = input.get("task_id").and_then(|value| value.as_str()) else {
            return;
        };
        let Some(run_id) = input
            .get("evidence_run_id")
            .and_then(|value| value.as_str())
        else {
            return;
        };
        let Some(evidence_ids) = input.get("evidence_ids") else {
            return;
        };
        let Ok(evidence_ids) = serde_json::from_value::<Vec<String>>(evidence_ids.clone()) else {
            return;
        };
        let Ok(persisted_tasks) = store.load_plan_tasks(&self.config.session_id) else {
            return;
        };
        let Some(task) = persisted_tasks.iter().find(|task| task.task_id == task_id) else {
            return;
        };
        let Some(authority) = self.plan_approval_authority.as_ref() else {
            return;
        };
        let Ok(resolved) = store.resolve_required_evidence(
            authority,
            &self.config.session_id,
            run_id,
            &evidence_ids,
            &task.required_evidence,
        ) else {
            return;
        };
        for item in resolved {
            let exists = self
                .plan_execution_evidence
                .completion
                .iter()
                .any(|existing| {
                    existing.evidence_id == item.evidence_id && existing.run_id == item.run_id
                });
            if !exists {
                self.plan_execution_evidence.completion.push(item);
            }
        }
    }

    pub(super) fn reconcile_active_plan(&self) -> Result<Option<String>, String> {
        let Some(store) = self.plan_store.as_ref() else {
            return Ok(None);
        };
        let Some(plan) = store
            .load_latest_plan(&self.config.session_id)
            .map_err(|error| error.to_string())?
        else {
            return Ok(None);
        };
        if !matches!(plan.status, PlanStatus::Approved | PlanStatus::Executing) {
            return Ok(None);
        }
        let _ = store;
        // Checked terminal transitions and file mutations write reconciliation
        // with their durable source records in the same transaction. Rebuilding
        // it here from transient agent state would discard that authority.
        Ok(reconciliation_summary(&plan.reconciliation))
    }

    pub(super) fn plan_completion_block(&mut self) -> Option<String> {
        if let Err(error) = self.retry_pending_observation_failure() {
            return Some(format!(
                "Plan completion is blocked because a filesystem observation failure could not be durably persisted: {error}"
            ));
        }
        if let Some(failure) = &self.observation_failure_blocker {
            return Some(format!(
                "Plan completion is blocked because a filesystem observation failure was not durably persisted: {failure}"
            ));
        }
        match self.reconcile_active_plan() {
            Ok(Some(summary)) => Some(format!(
                "Plan completion is blocked. {summary} Complete every approved plan task with its required durable evidence and remove or explicitly plan extra file changes."
            )),
            Ok(None) => None,
            Err(error) => Some(format!(
                "Plan completion is blocked because reconciliation could not be persisted: {error}"
            )),
        }
    }
}

#[cfg(test)]
#[path = "plan_reconciliation_finalization_tests.rs"]
mod finalization_tests;
#[cfg(test)]
#[path = "plan_reconciliation_tests.rs"]
mod tests;
