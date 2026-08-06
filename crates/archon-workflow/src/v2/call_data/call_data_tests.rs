use super::{
    fanout_items_for_call, result_from_fanout_report, source_pack_value, v2_agent_request,
};
use crate::task_universe::{
    WorkflowV2DeliverableContract, WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask,
};
use crate::{
    WorkflowSpec, WorkflowV2AgentAdapter, WorkflowV2BranchOutcome, WorkflowV2CallExecution,
    WorkflowV2CallRecord, WorkflowV2Evidence, WorkflowV2EvidenceKind, WorkflowV2FanoutReport,
    WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2HostOptions, WorkflowV2Result,
    WorkflowV2ResultStore, WorkflowV2Status, WorkflowV2TaskCoverage, WorkflowV2TaskCoverageStatus,
    WorkflowV2WriteMode,
};
use std::collections::BTreeMap;

#[path = "call_data_tests_a.rs"]
mod call_data_tests_a;
#[path = "call_data_tests_b.rs"]
mod call_data_tests_b;
#[path = "call_data_tests_c.rs"]
mod call_data_tests_c;
use call_data_tests_c::*;
