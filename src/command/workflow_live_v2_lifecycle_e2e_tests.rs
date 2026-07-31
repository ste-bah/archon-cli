use std::collections::BTreeMap;
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use archon_pipeline::runner::{AgentExecutionRequest, LlmClient, LlmResponse};
use archon_tui::event_channel::bounded_tui_event_channel;
use archon_workflow::{
    BranchFailureKind, WorkflowSpec, WorkflowStore, WorkflowV2AgentAdapter,
    WorkflowV2BranchOutcome, WorkflowV2CommandKind, WorkflowV2CommandRecord,
    WorkflowV2CommandStatus, WorkflowV2Result, WorkflowV2ResultStore, WorkflowV2Status,
};

use super::*;
use crate::command::workflow_live::workflow_live_task_universe::{
    WorkflowV2DeliverableContract, WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask,
};
use crate::command::workflow_live::workflow_live_v2::workflow_live_v2_verification;

include!("workflow_live_v2_lifecycle_e2e_tests_a.rs");
include!("workflow_live_v2_lifecycle_e2e_tests_b.rs");
include!("workflow_live_v2_lifecycle_e2e_tests_c.rs");
include!("workflow_live_v2_lifecycle_e2e_tests_d.rs");
