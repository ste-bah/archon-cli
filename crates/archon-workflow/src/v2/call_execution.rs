//! The unit of work handed to the v2 scheduler and the result store.
//!
//! This type used to live alongside `WorkflowV2Runtime` in `v2/runtime.rs`.
//! The runtime was superseded by [`super::scheduler::WorkflowV2Scheduler`] and
//! by the hand-rolled serial fan-out in the CLI, so it was removed; the call
//! execution record outlived it and now stands on its own.

use serde::{Deserialize, Serialize};

use super::WorkflowV2HostCall;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowV2CallExecution {
    pub call: WorkflowV2HostCall,
    #[serde(default)]
    pub input: serde_json::Value,
    #[serde(default)]
    pub depends_on: Vec<String>,
}
