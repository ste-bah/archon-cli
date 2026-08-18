use std::collections::BTreeMap;

use cozo::{DataValue, MultiTransaction};

use super::db_err;
use crate::plan_models::{PersistedPlanTask, PlanApprovalRecord, PlanDocument};

pub(super) enum PlanWrite {
    MaterializationClaim(BTreeMap<String, DataValue>),
    Plan(BTreeMap<String, DataValue>),
    Approval(BTreeMap<String, DataValue>),
    TaskInsert(BTreeMap<String, DataValue>),
}

impl PlanWrite {
    pub(super) fn materialization_claim(
        session_id: &str,
        plan: &PlanDocument,
    ) -> Result<Self, std::io::Error> {
        let mut params = BTreeMap::new();
        params.insert("session_id".to_string(), DataValue::from(session_id));
        params.insert("plan_id".to_string(), DataValue::from(plan.id.as_str()));
        params.insert("generation".to_string(), DataValue::from("materialized"));
        Ok(Self::MaterializationClaim(params))
    }

    pub(super) fn plan(session_id: &str, plan: &PlanDocument) -> Result<Self, std::io::Error> {
        let mut params = BTreeMap::new();
        params.insert("session_id".to_string(), DataValue::from(session_id));
        params.insert("plan_id".to_string(), DataValue::from(plan.id.as_str()));
        params.insert(
            "plan_json".to_string(),
            DataValue::from(plan.to_json().as_str()),
        );
        let updated_at = chrono::Utc::now().to_rfc3339();
        params.insert(
            "updated_at".to_string(),
            DataValue::from(updated_at.as_str()),
        );
        Ok(Self::Plan(params))
    }

    pub(super) fn approval(record: &PlanApprovalRecord) -> Result<Self, std::io::Error> {
        let mut params = BTreeMap::new();
        params.insert(
            "session_id".to_string(),
            DataValue::from(record.session_id.as_str()),
        );
        params.insert(
            "plan_id".to_string(),
            DataValue::from(record.plan_id.as_str()),
        );
        params.insert(
            "decided_at".to_string(),
            DataValue::from(record.approval.decided_at.as_str()),
        );
        let approval_json = serde_json::to_string(&record.approval).map_err(db_err)?;
        params.insert(
            "approval_json".to_string(),
            DataValue::from(approval_json.as_str()),
        );
        Ok(Self::Approval(params))
    }

    pub(super) fn task(session_id: &str, task: &PersistedPlanTask) -> Result<Self, std::io::Error> {
        let mut params = BTreeMap::new();
        params.insert("session_id".to_string(), DataValue::from(session_id));
        params.insert(
            "task_id".to_string(),
            DataValue::from(task.task_id.as_str()),
        );
        params.insert(
            "plan_id".to_string(),
            DataValue::from(task.plan_id.as_str()),
        );
        params.insert(
            "plan_step".to_string(),
            DataValue::from(i64::from(task.plan_step)),
        );
        let task_json = serde_json::to_string(task).map_err(db_err)?;
        params.insert("task_json".to_string(), DataValue::from(task_json.as_str()));
        params.insert(
            "updated_at".to_string(),
            DataValue::from(task.updated_at.as_str()),
        );
        Ok(Self::TaskInsert(params))
    }

    pub(super) fn run(self, transaction: &MultiTransaction) -> Result<(), std::io::Error> {
        let result = match self {
            Self::MaterializationClaim(params) => transaction.run_script(
                "?[session_id, plan_id, generation] <- [[$session_id, $plan_id, $generation]]
                 :insert plan_materializations {session_id, plan_id => generation}",
                params,
            ),
            Self::Plan(params) => transaction.run_script(
                "?[session_id, plan_id, plan_json, updated_at] <- [[$session_id, $plan_id, $plan_json, $updated_at]]
                 :put plans {session_id, plan_id => plan_json, updated_at}",
                params,
            ),
            Self::Approval(params) => transaction.run_script(
                "?[session_id, plan_id, decided_at, approval_json] <- [[$session_id, $plan_id, $decided_at, $approval_json]]
                 :insert plan_approval_events {session_id, plan_id, decided_at => approval_json}",
                params,
            ),
            Self::TaskInsert(params) => transaction.run_script(
                "?[session_id, task_id, plan_id, plan_step, task_json, updated_at] <- [[$session_id, $task_id, $plan_id, $plan_step, $task_json, $updated_at]]
                 :insert plan_tasks {session_id, task_id => plan_id, plan_step, task_json, updated_at}",
                params,
            ),
        };
        result.map_err(db_err)?;
        Ok(())
    }
}
