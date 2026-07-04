use std::collections::BTreeSet;
use std::future::Future;

use serde::{Deserialize, Serialize};

use crate::{WorkflowError, WorkflowResult};

use super::{
    WorkflowV2CallRecord, WorkflowV2HostCall, WorkflowV2Result, WorkflowV2ResultStore,
    WorkflowV2ResumeDecision, WorkflowV2Status,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowV2CallExecution {
    pub call: WorkflowV2HostCall,
    #[serde(default)]
    pub input: serde_json::Value,
    #[serde(default)]
    pub depends_on: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowV2RunSummary {
    pub status: WorkflowV2Status,
    pub completed: usize,
    pub executed: usize,
    pub reused: usize,
    pub failed_call: Option<String>,
    pub next_action: Option<String>,
}

#[derive(Debug, Clone)]
pub struct WorkflowV2Runtime {
    store: WorkflowV2ResultStore,
}

impl WorkflowV2Runtime {
    pub fn new(store: WorkflowV2ResultStore) -> Self {
        Self { store }
    }

    pub fn store(&self) -> &WorkflowV2ResultStore {
        &self.store
    }

    pub fn run_serial<F>(
        &self,
        executions: &[WorkflowV2CallExecution],
        mut execute: F,
    ) -> WorkflowResult<WorkflowV2RunSummary>
    where
        F: FnMut(&WorkflowV2CallExecution) -> WorkflowResult<WorkflowV2Result>,
    {
        let mut checkpoint = self.store.load_checkpoint()?.unwrap_or_default();
        let mut completed = 0;
        let mut executed = 0;
        let mut reused = 0;
        let mut dirty_call_ids = BTreeSet::new();
        let mut final_status = WorkflowV2Status::Accepted;

        for execution in executions {
            let input_hash = stable_input_hash(&execution.input);
            let dependency_reran = execution
                .depends_on
                .iter()
                .any(|dependency| dirty_call_ids.contains(dependency));
            if !dependency_reran {
                match self.resume_decision(&execution.call.id, &input_hash)? {
                    WorkflowV2ResumeDecision::ReuseCachedResult => {
                        if let Some(record) = self.store.load_call_record(&execution.call.id)? {
                            final_status = merge_status(final_status, record.status);
                        }
                        reused += 1;
                        completed += 1;
                        checkpoint.mark_completed(&execution.call.id);
                        self.store.save_checkpoint(&checkpoint)?;
                        continue;
                    }
                    WorkflowV2ResumeDecision::Execute => {}
                }
            }

            let attempt = self
                .store
                .load_call_record(&execution.call.id)?
                .map_or(1, |record| record.attempt.saturating_add(1));
            let result = execute(execution)?;
            result.validate().map_err(|err| {
                WorkflowError::SpecInvalid(format!(
                    "invalid workflow v2 result for call '{}': {err}",
                    execution.call.id
                ))
            })?;

            let status = result.status;
            final_status = merge_status(final_status, status);
            let record = WorkflowV2CallRecord::new(
                self.store.run_id(),
                execution.call.clone(),
                attempt,
                input_hash,
                result,
                execution.depends_on.clone(),
            );
            self.store.save_call_record(&record)?;
            if is_checkpointable_status(status) {
                checkpoint.mark_completed(&execution.call.id);
                completed += 1;
            } else {
                checkpoint.remove_completed_call(&execution.call.id);
            }
            self.store.save_checkpoint(&checkpoint)?;
            executed += 1;
            dirty_call_ids.insert(execution.call.id.clone());

            if is_terminal_stop_status(status) {
                return Ok(WorkflowV2RunSummary {
                    status,
                    completed,
                    executed,
                    reused,
                    failed_call: Some(execution.call.id.clone()),
                    next_action: Some(next_action_for_terminal_call(&execution.call.id)),
                });
            }
        }

        Ok(WorkflowV2RunSummary {
            status: final_status,
            completed,
            executed,
            reused,
            failed_call: None,
            next_action: None,
        })
    }

    pub async fn run_serial_async<F, Fut>(
        &self,
        executions: &[WorkflowV2CallExecution],
        execute: F,
    ) -> WorkflowResult<WorkflowV2RunSummary>
    where
        F: FnMut(WorkflowV2CallExecution) -> Fut,
        Fut: Future<Output = WorkflowResult<WorkflowV2Result>>,
    {
        self.run_serial_async_with_prepared_inputs(
            executions,
            |execution| async move { Ok(execution) },
            execute,
        )
        .await
    }

    pub async fn run_serial_async_with_prepared_inputs<P, PFut, F, Fut>(
        &self,
        executions: &[WorkflowV2CallExecution],
        mut prepare: P,
        mut execute: F,
    ) -> WorkflowResult<WorkflowV2RunSummary>
    where
        P: FnMut(WorkflowV2CallExecution) -> PFut,
        PFut: Future<Output = WorkflowResult<WorkflowV2CallExecution>>,
        F: FnMut(WorkflowV2CallExecution) -> Fut,
        Fut: Future<Output = WorkflowResult<WorkflowV2Result>>,
    {
        let mut checkpoint = self.store.load_checkpoint()?.unwrap_or_default();
        let mut completed = 0;
        let mut executed = 0;
        let mut reused = 0;
        let mut dirty_call_ids = BTreeSet::new();
        let mut final_status = WorkflowV2Status::Accepted;

        for execution in executions {
            let execution = prepare(execution.clone()).await?;
            let input_hash = stable_input_hash(&execution.input);
            let dependency_reran = execution
                .depends_on
                .iter()
                .any(|dependency| dirty_call_ids.contains(dependency));
            if !dependency_reran {
                match self.resume_decision(&execution.call.id, &input_hash)? {
                    WorkflowV2ResumeDecision::ReuseCachedResult => {
                        if let Some(record) = self.store.load_call_record(&execution.call.id)? {
                            final_status = merge_status(final_status, record.status);
                        }
                        reused += 1;
                        completed += 1;
                        checkpoint.mark_completed(&execution.call.id);
                        self.store.save_checkpoint(&checkpoint)?;
                        continue;
                    }
                    WorkflowV2ResumeDecision::Execute => {}
                }
            }

            let attempt = self
                .store
                .load_call_record(&execution.call.id)?
                .map_or(1, |record| record.attempt.saturating_add(1));
            let result = execute(execution.clone()).await?;
            result.validate().map_err(|err| {
                WorkflowError::SpecInvalid(format!(
                    "invalid workflow v2 result for call '{}': {err}",
                    execution.call.id
                ))
            })?;

            let status = result.status;
            final_status = merge_status(final_status, status);
            let record = WorkflowV2CallRecord::new(
                self.store.run_id(),
                execution.call.clone(),
                attempt,
                input_hash,
                result,
                execution.depends_on.clone(),
            );
            self.store.save_call_record(&record)?;
            if is_checkpointable_status(status) {
                checkpoint.mark_completed(&execution.call.id);
                completed += 1;
            } else {
                checkpoint.remove_completed_call(&execution.call.id);
            }
            self.store.save_checkpoint(&checkpoint)?;
            executed += 1;
            dirty_call_ids.insert(execution.call.id.clone());

            if is_terminal_stop_status(status) {
                return Ok(WorkflowV2RunSummary {
                    status,
                    completed,
                    executed,
                    reused,
                    failed_call: Some(execution.call.id.clone()),
                    next_action: Some(next_action_for_terminal_call(&execution.call.id)),
                });
            }
        }

        Ok(WorkflowV2RunSummary {
            status: final_status,
            completed,
            executed,
            reused,
            failed_call: None,
            next_action: None,
        })
    }

    fn resume_decision(
        &self,
        call_id: &str,
        input_hash: &str,
    ) -> WorkflowResult<WorkflowV2ResumeDecision> {
        let Some(record) = self.store.load_call_record(call_id)? else {
            return Ok(WorkflowV2ResumeDecision::Execute);
        };
        if record.is_reusable_for(input_hash) {
            Ok(WorkflowV2ResumeDecision::ReuseCachedResult)
        } else {
            Ok(WorkflowV2ResumeDecision::Execute)
        }
    }
}

fn is_checkpointable_status(status: WorkflowV2Status) -> bool {
    matches!(status, WorkflowV2Status::Accepted | WorkflowV2Status::Noop)
}

fn is_terminal_stop_status(status: WorkflowV2Status) -> bool {
    matches!(
        status,
        WorkflowV2Status::Failed | WorkflowV2Status::Cancelled
    )
}

fn merge_status(left: WorkflowV2Status, right: WorkflowV2Status) -> WorkflowV2Status {
    if status_precedence(right) > status_precedence(left) {
        right
    } else {
        left
    }
}

fn status_precedence(status: WorkflowV2Status) -> u8 {
    match status {
        WorkflowV2Status::Cancelled => 7,
        WorkflowV2Status::Failed => 6,
        WorkflowV2Status::Blocked => 5,
        WorkflowV2Status::NeedsReview => 4,
        WorkflowV2Status::Running => 3,
        WorkflowV2Status::Pending => 2,
        WorkflowV2Status::Accepted | WorkflowV2Status::Noop => 1,
    }
}

fn next_action_for_terminal_call(call_id: &str) -> String {
    format!(
        "choose one: /workflow restart-stage <run-id> {call_id}, \
         /workflow restart-item <run-id> {call_id} <item-id> when a single branch failed, \
         or fix the recorded evidence/artifact and /workflow resume --live <run-id>"
    )
}

fn stable_input_hash(input: &serde_json::Value) -> String {
    let bytes = serde_json::to_vec(input).unwrap_or_default();
    blake3::hash(&bytes).to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_hash_is_stable_for_same_json_value() {
        let left = serde_json::json!({"a": 1, "b": ["x", "y"]});
        let right = serde_json::json!({"a": 1, "b": ["x", "y"]});
        assert_eq!(stable_input_hash(&left), stable_input_hash(&right));
    }

    #[tokio::test]
    async fn async_serial_runtime_executes_and_reuses_results() {
        let temp = tempfile::tempdir().expect("tempdir");
        let runtime = WorkflowV2Runtime::new(WorkflowV2ResultStore::new(temp.path()));
        let executions = vec![WorkflowV2CallExecution {
            call: WorkflowV2HostCall {
                id: "inspect".to_string(),
                method: super::super::WorkflowV2HostMethod::Agent,
                write_mode: None,
                options: Default::default(),
            },
            input: serde_json::json!({"task": "inspect"}),
            depends_on: Vec::new(),
        }];

        let first = runtime
            .run_serial_async(&executions, |_execution| async {
                let mut result = WorkflowV2Result::accepted("done");
                result.evidence.push(super::super::WorkflowV2Evidence::new(
                    super::super::WorkflowV2EvidenceKind::Inspection,
                    "read source",
                ));
                Ok(result)
            })
            .await
            .expect("first run");
        let second = runtime
            .run_serial_async(&executions, |_execution| async {
                panic!("cached accepted result should be reused")
            })
            .await
            .expect("second run");

        assert_eq!(first.executed, 1);
        assert_eq!(second.executed, 0);
        assert_eq!(second.reused, 1);
    }

    #[tokio::test]
    async fn async_serial_runtime_does_not_checkpoint_failed_result() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = WorkflowV2ResultStore::new(temp.path());
        let runtime = WorkflowV2Runtime::new(store.clone());
        let executions = vec![WorkflowV2CallExecution {
            call: WorkflowV2HostCall {
                id: "verify".to_string(),
                method: super::super::WorkflowV2HostMethod::Agent,
                write_mode: None,
                options: Default::default(),
            },
            input: serde_json::json!({"task": "verify"}),
            depends_on: Vec::new(),
        }];

        let summary = runtime
            .run_serial_async(&executions, |_execution| async {
                Ok(WorkflowV2Result {
                    status: WorkflowV2Status::Failed,
                    summary: "verification failed".to_string(),
                    ..WorkflowV2Result::default()
                })
            })
            .await
            .expect("run");

        assert_eq!(summary.status, WorkflowV2Status::Failed);
        assert_eq!(summary.completed, 0);
        assert_eq!(
            store
                .load_checkpoint()
                .expect("load checkpoint")
                .expect("checkpoint")
                .completed_call_ids,
            Vec::<String>::new()
        );
    }
}
