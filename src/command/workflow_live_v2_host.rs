use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use archon_workflow::{
    WorkflowError, WorkflowV2Artifact, WorkflowV2CallExecution, WorkflowV2Evidence,
    WorkflowV2EvidenceKind, WorkflowV2FinalReport, WorkflowV2FinalReportBuilder,
    WorkflowV2HostMethod, WorkflowV2ReportPaths, WorkflowV2ResidualGap, WorkflowV2Result,
    WorkflowV2ResultStore, WorkflowV2Status,
};

use archon_workflow::task_universe::WorkflowV2TaskUniverse;
use archon_workflow::v2::completion_credit::{
    CompletionCredit, noop_acceptance_criteria_satisfied,
};

#[path = "workflow_live_v2_host_local.rs"]
mod workflow_live_v2_host_local;
pub(crate) use workflow_live_v2_host_local::*;

#[path = "workflow_live_v2_host_final.rs"]
mod workflow_live_v2_host_final;
pub(super) use workflow_live_v2_host_final::*;

#[path = "workflow_live_v2_final_accounting.rs"]
mod workflow_live_v2_final_accounting;
pub(crate) use workflow_live_v2_final_accounting::*;

#[path = "workflow_live_v2_host_support.rs"]
mod workflow_live_v2_host_support;
use workflow_live_v2_host_support::*;
#[path = "workflow_live_v2_host_support_a.rs"]
mod workflow_live_v2_host_support_a;
pub(crate) use workflow_live_v2_host_support_a::*;
#[path = "workflow_live_v2_host_support_b.rs"]
mod workflow_live_v2_host_support_b;
pub(crate) use workflow_live_v2_host_support_b::*;

#[path = "workflow_live_v2_host_blocker.rs"]
mod workflow_live_v2_host_blocker;
pub(super) use workflow_live_v2_host_blocker::*;
