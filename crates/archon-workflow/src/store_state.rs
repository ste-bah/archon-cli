//! Run state writes enter the captured-owner boundary at the write itself.
use super::*;
impl WorkflowStore {
    pub fn save_state(&self, run: &WorkflowRun) -> WorkflowResult<()> {
        self.with_writer(&run.id, || {
            if let Ok(current) = self.load_state(&run.id)
                && current.generation > run.generation
            {
                return Err(WorkflowError::StateCorrupt(format!(
                    "stale workflow state generation for {}: local {}, current {}",
                    run.id, run.generation, current.generation
                )));
            }
            let target = self.state_path(&run.id);
            let tmp = target.with_extension("json.tmp");
            let json = serde_json::to_vec_pretty(run)?;
            write_atomic(&tmp, &target, &json)?;
            crate::durable_io::sync_dir(
                target.parent().unwrap_or_else(|| std::path::Path::new(".")),
            )
        })
    }

    pub(crate) fn restore_state_after_failed_transition(
        &self,
        prior: &WorkflowRun,
        failed_generation: u64,
    ) -> WorkflowResult<()> {
        self.with_writer(&prior.id, || {
        let current = self.load_state(&prior.id)?;
        if current.generation != failed_generation {
            return Err(WorkflowError::StateCorrupt(format!(
                "cannot restore workflow {} after failed transition: expected generation {}, found {}",
                prior.id, failed_generation, current.generation
            )));
        }
        let target = self.state_path(&prior.id);
        let tmp = target.with_extension("json.tmp");
        let json = serde_json::to_vec_pretty(prior)?;
        write_atomic(&tmp, &target, &json)?;
        crate::durable_io::sync_dir(target.parent().unwrap_or_else(|| std::path::Path::new(".")))
        })
    }

    pub fn save_state_preserving_control(&self, run: &WorkflowRun) -> WorkflowResult<()> {
        self.with_writer(&run.id, || {
            let mut writable = run.clone();
            if let Ok(current) = self.load_state(&run.id)
                && current.generation > run.generation
            {
                match current.status {
                    RunStatus::Paused | RunStatus::Cancelled => {
                        writable.status = current.status;
                        writable.generation = current.generation;
                        writable.executor_generation = current.executor_generation;
                        writable.updated_at = current.updated_at;
                        for (stage_id, current_stage) in current.stages {
                            if let Some(stage) = writable.stages.get_mut(&stage_id)
                                && matches!(
                                    current_stage.status,
                                    crate::run::StageStatus::Paused
                                        | crate::run::StageStatus::Cancelled
                                )
                            {
                                *stage = current_stage;
                            }
                        }
                        for (item_id, current_item) in current.items {
                            if matches!(current_item.status, crate::run::StageStatus::Cancelled) {
                                writable.items.insert(item_id, current_item);
                            }
                        }
                    }
                    _ => {
                        return Err(WorkflowError::StateCorrupt(format!(
                            "stale workflow state generation for {}: local {}, current {}",
                            run.id, run.generation, current.generation
                        )));
                    }
                }
            }
            let target = self.state_path(&writable.id);
            let tmp = target.with_extension("json.tmp");
            let json = serde_json::to_vec_pretty(&writable)?;
            write_atomic(&tmp, &target, &json)?;
            crate::durable_io::sync_dir(
                target.parent().unwrap_or_else(|| std::path::Path::new(".")),
            )
        })
    }
}

#[cfg(test)]
#[path = "store_state_tests.rs"]
mod tests;
