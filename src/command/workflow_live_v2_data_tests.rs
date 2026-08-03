use std::collections::BTreeMap;
use archon_workflow::{
    WorkflowSpec, WorkflowV2BranchOutcome, WorkflowV2CallExecution, WorkflowV2CallRecord,
    WorkflowV2Evidence, WorkflowV2EvidenceKind, WorkflowV2FanoutReport, WorkflowV2HostCall,
    WorkflowV2HostMethod, WorkflowV2HostOptions, WorkflowV2Result, WorkflowV2ResultStore,
    WorkflowV2Status, WorkflowV2TaskCoverage, WorkflowV2TaskCoverageStatus, WorkflowV2WriteMode,
    WorkflowV2AgentAdapter,
};
use super::super::workflow_live_task_universe::{
    WorkflowV2DeliverableContract, WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask,
};
use super::workflow_live_v2_data::{
    fanout_items_for_call, result_from_fanout_report, source_pack_value, v2_agent_request,
};

#[path = "workflow_live_v2_data_tests_a.rs"]
mod workflow_live_v2_data_tests_a;
use workflow_live_v2_data_tests_a::*;
#[path = "workflow_live_v2_data_tests_b.rs"]
mod workflow_live_v2_data_tests_b;
use workflow_live_v2_data_tests_b::*;
#[path = "workflow_live_v2_data_tests_c.rs"]
mod workflow_live_v2_data_tests_c;
use workflow_live_v2_data_tests_c::*;
