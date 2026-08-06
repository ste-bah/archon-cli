// A recording `LifecycleHost` for tests that assert on the SHAPE of the review
// loop rather than on one policy function.
//
// Round bounding is a property of the loop, not of any value it computes:
// "assignment_invalid stops after one round" and "the ordinary loop still stops
// after six" are both claims about how many times a stage ran, and only a host
// that counts calls can witness them. Asserting the final status instead would
// pass just as happily against a loop that spent the whole budget first.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::Value;

use crate::lifecycle_host_port::LifecycleHost;
use crate::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};
use crate::v2::result_store::WorkflowV2CallRecord;

use super::{LifecycleDriver, LifecycleLimits};

/// `(method, call id) -> reply`. Every call the driver makes is answered here,
/// so a stage the test did not anticipate shows up as an explicit panic rather
/// than as a silently empty result that the driver would read as a clean gate.
pub(crate) type Responder = Box<dyn Fn(&str, &str) -> Value + Send + Sync>;

pub(crate) struct RecordingHost {
    calls: Mutex<Vec<(String, String)>>,
    responder: Responder,
}

impl RecordingHost {
    pub(crate) fn new(responder: Responder) -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(Vec::new()),
            responder,
        })
    }

    pub(crate) fn call_ids(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .map(|(_, id)| id.clone())
            .collect()
    }

    pub(crate) fn ids_starting_with(&self, prefix: &str) -> Vec<String> {
        self.call_ids()
            .into_iter()
            .filter(|id| id.starts_with(prefix))
            .collect()
    }

    pub(crate) fn count_starting_with(&self, prefix: &str) -> usize {
        self.ids_starting_with(prefix).len()
    }
}

#[async_trait]
impl LifecycleHost for RecordingHost {
    async fn execute(&self, method: String, payload: String) -> crate::WorkflowResult<String> {
        let payload: Value = serde_json::from_str(&payload)?;
        let id = payload
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        self.calls
            .lock()
            .unwrap()
            .push((method.clone(), id.clone()));
        Ok((self.responder)(&method, &id).to_string())
    }

    fn load_call_record(
        &self,
        _call_id: &str,
    ) -> crate::WorkflowResult<Option<WorkflowV2CallRecord>> {
        Ok(None)
    }

    fn pack_reduce_source(&self, source: &Value) -> Value {
        source.clone()
    }
}

/// One-task universe. One is enough: every property under test is about how
/// many times the loop ran, and a second task only multiplies the fixture.
pub(crate) fn one_task_universe() -> WorkflowV2TaskUniverse {
    WorkflowV2TaskUniverse {
        schema_version: "1".to_string(),
        source_roots: vec!["tasks".to_string()],
        tasks: vec![WorkflowV2TaskUniverseTask {
            canonical_task_id: "TASK-001".to_string(),
            source_path: "tasks/TASK-001.md".to_string(),
            title: Some("the task under review".to_string()),
            acceptance_criteria: vec!["it does the thing".to_string()],
            ..WorkflowV2TaskUniverseTask::default()
        }],
    }
}

pub(crate) fn driver(host: Arc<RecordingHost>) -> LifecycleDriver {
    LifecycleDriver::new(
        host,
        one_task_universe(),
        None,
        None,
        Value::Null,
        std::collections::BTreeSet::new(),
        LifecycleLimits {
            max_repair_iterations: 1,
            max_investigation_iterations: 1,
            implementation_wave_max_parallelism: Some(1),
        },
    )
}

/// The replies every path through `run_review_and_final_gates` needs before it
/// reaches the review round itself.
pub(crate) fn preamble_reply(id: &str) -> Option<Value> {
    match id {
        "artifact-inventory" => Some(serde_json::json!({ "status": "accepted", "items": [] })),
        "save-artifact-inventory" => Some(serde_json::json!({ "status": "accepted" })),
        _ => None,
    }
}

pub(crate) fn accepted_report() -> Value {
    serde_json::json!({ "status": "accepted", "summary": "report recorded" })
}

/// A per-task review finding already on the evidence bundle, as a wave-time
/// reviewer would have left it. Round 1 reuses these rather than re-reviewing.
pub(crate) fn seeded_review_evidence(findings: Vec<Value>) -> Value {
    serde_json::json!({
        "kind": "adversarial-review-task",
        "reviewRound": "wave-1",
        "reviewedTaskIds": ["TASK-001"],
        "findings": findings,
    })
}
