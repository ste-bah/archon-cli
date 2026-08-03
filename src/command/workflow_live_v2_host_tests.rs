use super::workflow_live_v2_host::{
    artifact_path_exists, execute_local_host_call, reconcile_final_task_statuses,
    validated_completion_credit,
};
use archon_workflow::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};
use archon_workflow::{
    WorkflowV2BranchOutcome, WorkflowV2CallExecution, WorkflowV2CallRecord, WorkflowV2CommandKind,
    WorkflowV2CommandRecord, WorkflowV2CommandStatus, WorkflowV2Evidence, WorkflowV2EvidenceKind,
    WorkflowV2FinalReport, WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2HostOptions,
    WorkflowV2ResidualGap, WorkflowV2Result, WorkflowV2ResultStore, WorkflowV2Status,
    WorkflowV2TaskCompletionEvidence, WorkflowV2TaskCompletionEvidenceKind, WorkflowV2TaskCoverage,
    WorkflowV2TaskCoverageStatus,
};
use std::path::Path;

#[path = "workflow_live_v2_host_tests_a.rs"]
mod workflow_live_v2_host_tests_a;
use workflow_live_v2_host_tests_a::*;
#[path = "workflow_live_v2_host_tests_b.rs"]
mod workflow_live_v2_host_tests_b;
use workflow_live_v2_host_tests_b::*;
#[path = "workflow_live_v2_host_tests_c.rs"]
mod workflow_live_v2_host_tests_c;
use workflow_live_v2_host_tests_c::*;
