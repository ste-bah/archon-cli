//! The host calls a generated workflow executes without an agent.
//!
//! `checkpoint`, `saveArtifact`, `requireArtifact`, `finalReport`,
//! `qualityGate` and `humanGate` are decided entirely from the V2 result store,
//! the authoritative task universe and the filesystem — no LLM, no CLI, no
//! terminal. That makes them a property of the runtime, so they live beside the
//! store, the report builder and the completion credit they read rather than in
//! the binary that happens to drive them.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::task_universe::WorkflowV2TaskUniverse;
use crate::v2::completion_credit::{CompletionCredit, noop_acceptance_criteria_satisfied};
use crate::{
    WorkflowError, WorkflowV2Artifact, WorkflowV2CallExecution, WorkflowV2Evidence,
    WorkflowV2EvidenceKind, WorkflowV2FinalReport, WorkflowV2FinalReportBuilder,
    WorkflowV2HostMethod, WorkflowV2ReportPaths, WorkflowV2ResidualGap, WorkflowV2Result,
    WorkflowV2ResultStore, WorkflowV2Status,
};

#[path = "local_host_calls.rs"]
mod local_host_calls;
// The one item outside this crate names: the generated-workflow driver in the
// bin crate dispatches every local method through it.
pub use local_host_calls::execute_local_host_call;

#[path = "local_host_final.rs"]
mod local_host_final;
use local_host_final::*;

#[path = "local_host_final_accounting.rs"]
mod local_host_final_accounting;
use local_host_final_accounting::*;

#[path = "local_host_support_a.rs"]
mod local_host_support_a;
use local_host_support_a::*;
#[path = "local_host_support_b.rs"]
mod local_host_support_b;
use local_host_support_b::*;

#[path = "local_host_blocker.rs"]
mod local_host_blocker;
use local_host_blocker::*;

#[cfg(test)]
#[path = "local_host_tests.rs"]
mod tests;
