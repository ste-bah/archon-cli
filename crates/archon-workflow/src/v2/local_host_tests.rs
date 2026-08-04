use super::{
    artifact_path_exists, execute_local_host_call, reconcile_final_task_statuses,
    validated_completion_credit,
};
use crate::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};
use crate::{
    WorkflowV2BranchOutcome, WorkflowV2CallExecution, WorkflowV2CallRecord, WorkflowV2CommandKind,
    WorkflowV2CommandRecord, WorkflowV2CommandStatus, WorkflowV2Evidence, WorkflowV2EvidenceKind,
    WorkflowV2FinalReport, WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2HostOptions,
    WorkflowV2ResidualGap, WorkflowV2Result, WorkflowV2ResultStore, WorkflowV2Status,
    WorkflowV2TaskCompletionEvidence, WorkflowV2TaskCompletionEvidenceKind, WorkflowV2TaskCoverage,
    WorkflowV2TaskCoverageStatus,
};
use std::path::Path;

#[path = "local_host_tests_a.rs"]
mod local_host_tests_a;
#[path = "local_host_tests_b.rs"]
mod local_host_tests_b;
// `_c` is the only split that exports shared helpers (`execution`, the two task
// universes); `_a` and `_b` hold tests only, so they are declared but not
// glob-imported.
#[path = "local_host_tests_c.rs"]
mod local_host_tests_c;
use local_host_tests_c::*;
