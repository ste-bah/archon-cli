use std::collections::BTreeMap;

use cozo::{DataValue, ScriptMutability};

use super::{PersistedPlanTask, PlanStore, db_err};
#[cfg(test)]
use crate::plan_models::PlanStepStatus;

impl PlanStore {
    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn save_plan_task_fixture(
        &self,
        session_id: &str,
        task: &PersistedPlanTask,
    ) -> Result<(), std::io::Error> {
        self.save_unmaterialized_plan_task(session_id, task)
    }

    #[cfg(test)]
    #[doc(hidden)]
    pub(crate) fn save_plan_task_with_step_status(
        &self,
        _session_id: &str,
        _task: &PersistedPlanTask,
        _status: PlanStepStatus,
    ) -> Result<(), std::io::Error> {
        Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "plan task status persistence is not publicly writable",
        ))
    }

    pub fn load_plan_tasks(
        &self,
        session_id: &str,
    ) -> Result<Vec<PersistedPlanTask>, std::io::Error> {
        let mut params = BTreeMap::new();
        params.insert("sid".to_string(), DataValue::from(session_id));
        let rows = self
            .db
            .run_script(
                "?[task_json] := *plan_tasks{session_id, task_json}, session_id = $sid :sort task_json",
                params,
                ScriptMutability::Immutable,
            )
            .map_err(db_err)?;
        rows.rows
            .iter()
            .map(|row| serde_json::from_str(row[0].get_str().unwrap_or("")).map_err(db_err))
            .collect()
    }

    #[cfg(test)]
    #[doc(hidden)]
    pub(crate) fn update_plan_task_status(
        &self,
        _session_id: &str,
        _task_id: &str,
        _status: &str,
    ) -> Result<(), std::io::Error> {
        Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "plan task status persistence is not publicly writable",
        ))
    }
}
