use std::collections::{BTreeMap, BTreeSet};

use archon_workflow::{
    WorkflowV2CallExecution, WorkflowV2HostMethod, WorkflowV2SourceTargetExpansion,
    WorkflowV2SourceTaskGraph, WorkflowV2SourceTaskItem, WorkflowV2WriteMode,
};

use super::super::workflow_live_generated_contract::{
    GeneratedContractIssueKind, normalize_generated_item_value,
};
use super::super::workflow_live_task_universe::WorkflowV2TaskUniverse;
use super::workflow_live_v2_stable_json::stable_hash;
use super::workflow_live_v2_target_expansion::expand_declared_rust_module_targets;

#[path = "workflow_live_v2_source_graph_core.rs"]
mod workflow_live_v2_source_graph_core;
pub(crate) use workflow_live_v2_source_graph_core::*;

#[path = "workflow_live_v2_source_graph_build.rs"]
mod workflow_live_v2_source_graph_build;
pub(super) use workflow_live_v2_source_graph_build::*;

#[path = "workflow_live_v2_source_graph_fields.rs"]
mod workflow_live_v2_source_graph_fields;
pub(super) use workflow_live_v2_source_graph_fields::*;

#[path = "workflow_live_v2_source_graph_diagnostics.rs"]
mod workflow_live_v2_source_graph_diagnostics;
pub(super) use workflow_live_v2_source_graph_diagnostics::*;

#[path = "workflow_live_v2_source_graph_helpers.rs"]
mod workflow_live_v2_source_graph_helpers;
pub(super) use workflow_live_v2_source_graph_helpers::*;

#[cfg(test)]
#[path = "workflow_live_v2_source_graph_tests.rs"]
mod tests;
