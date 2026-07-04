use std::collections::{BTreeMap, BTreeSet};

use archon_workflow::{
    WorkflowV2CallExecution, WorkflowV2HostMethod, WorkflowV2SourceTargetExpansion,
    WorkflowV2SourceTaskGraph, WorkflowV2SourceTaskItem, WorkflowV2WriteMode,
};

use super::super::workflow_live_generated_contract::normalize_generated_item_value;
use super::super::workflow_live_task_universe::WorkflowV2TaskUniverse;
use super::workflow_live_v2_stable_json::stable_hash;
use super::workflow_live_v2_target_expansion::expand_declared_rust_module_targets;

include!("workflow_live_v2_source_graph_core.rs");

include!("workflow_live_v2_source_graph_build.rs");

include!("workflow_live_v2_source_graph_fields.rs");

include!("workflow_live_v2_source_graph_helpers.rs");

#[cfg(test)]
#[path = "workflow_live_v2_source_graph_tests.rs"]
mod tests;
