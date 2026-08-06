use super::{
    GeneratedV2RestartTarget, WorkflowHandler, cli_action, generated_v2_restart_target,
    invalidate_generated_v2_call, invalidate_generated_v2_item, restart_task_workflow,
    stage_id_for_task, status_text,
};
use crate::cli_args::WorkflowAction;
use crate::command::registry::CommandHandler;
use crate::command::test_support::{CtxBuilder, drain_tui_events};
use archon_tui::app::TuiEvent;
use archon_workflow::run::StageState;
use archon_workflow::{
    CommandAction, ProviderTier, RetryPolicy, RunStatus, StageKind, StageSpec, StageStatus,
    WorkflowBundle, WorkflowBundleOrigin, WorkflowRun, WorkflowSpec, WorkflowStore,
    WorkflowV2BranchOutcome, WorkflowV2CallRecord, WorkflowV2Evidence, WorkflowV2EvidenceKind,
    WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2HostOptions, WorkflowV2Result,
    WorkflowV2ResultStore, WorkflowV2SourceTaskGraph, WorkflowV2SourceTaskItem, WorkflowV2Status,
    WorkflowV2TaskCompletionEvidence, WorkflowV2TaskCompletionEvidenceKind, WorkflowV2WriteMode,
};
use serde_json::json;
use std::collections::BTreeMap;

#[path = "workflow_tests_a.rs"]
mod workflow_tests_a;
#[path = "workflow_tests_b.rs"]
mod workflow_tests_b;
use workflow_tests_b::*;
