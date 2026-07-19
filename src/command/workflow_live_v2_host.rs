use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use archon_workflow::{
    WorkflowError, WorkflowV2Artifact, WorkflowV2CallExecution, WorkflowV2Evidence,
    WorkflowV2EvidenceKind, WorkflowV2FinalReport, WorkflowV2FinalReportBuilder,
    WorkflowV2HostMethod, WorkflowV2ReportPaths, WorkflowV2ResidualGap, WorkflowV2Result,
    WorkflowV2ResultStore, WorkflowV2Status,
};

use super::workflow_live_task_universe::WorkflowV2TaskUniverse;
use super::workflow_live_v2_completion_credit::{
    CompletionCredit, noop_acceptance_criteria_satisfied,
};

include!("workflow_live_v2_host_local.rs");

include!("workflow_live_v2_host_final.rs");

include!("workflow_live_v2_final_accounting.rs");

include!("workflow_live_v2_host_support.rs");

include!("workflow_live_v2_host_blocker.rs");
