use std::collections::BTreeMap;
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::command::tui_workflow_ui_sink::default_workflow_ui_sink;
use anyhow::Result;
use archon_workflow::{
    BranchFailureKind, WorkflowSpec, WorkflowStore, WorkflowV2AgentAdapter,
    WorkflowV2BranchOutcome, WorkflowV2CommandKind, WorkflowV2CommandRecord,
    WorkflowV2CommandStatus, WorkflowV2Result, WorkflowV2ResultStore, WorkflowV2Status,
};
use archon_workflow::{WorkflowAgentCall, WorkflowAgentOutcome, WorkflowLlmClient};

use super::*;
// The lifecycle itself now lives in archon-workflow; what stays here is the
// end-to-end coverage that drives it over the real host stack.
use archon_workflow::v2::lifecycle_driver::{LifecycleDriver, LifecycleEvidence, LifecycleLimits};
use archon_workflow::v2::lifecycle_policy;

use crate::command::workflow_live::workflow_live_v2::workflow_live_v2_verification;
use archon_workflow::task_universe::{
    WorkflowV2DeliverableContract, WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask,
};

#[path = "workflow_live_v2_lifecycle_e2e_tests_a.rs"]
mod workflow_live_v2_lifecycle_e2e_tests_a;
use workflow_live_v2_lifecycle_e2e_tests_a::*;
#[path = "workflow_live_v2_lifecycle_e2e_tests_b.rs"]
mod workflow_live_v2_lifecycle_e2e_tests_b;
use workflow_live_v2_lifecycle_e2e_tests_b::*;
#[path = "workflow_live_v2_lifecycle_e2e_tests_c.rs"]
mod workflow_live_v2_lifecycle_e2e_tests_c;
use workflow_live_v2_lifecycle_e2e_tests_c::*;
#[path = "workflow_live_v2_lifecycle_e2e_tests_d.rs"]
mod workflow_live_v2_lifecycle_e2e_tests_d;
use workflow_live_v2_lifecycle_e2e_tests_d::*;
