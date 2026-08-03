use std::collections::BTreeSet;
use std::process::Command;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use archon_workflow::WorkflowLlmClient;
use archon_workflow::{
    CommandAction, GeneratedWorkflowKind, LifecycleAction, LifecycleController, ProviderTier,
    RunStatus, StageKind, StageRunRequest, WorkflowApprovalStore, WorkflowBundle,
    WorkflowBundleOrigin, WorkflowCommandRegistry, WorkflowLearningEvent,
    WorkflowLearningEvidenceRef, WorkflowSpec, WorkflowStageRunner, WorkflowStore,
    WorkflowV2HostCall, WorkflowV2Status, workflow_scaffold_hash,
};
use serde_json::json;

use super::workflow_live_runner::PipelineWorkflowRunner;
use super::workflow_live_test_support::{
    AlwaysInvalidItemsAgentClient, BlockedInvalidItemsAgentClient, CompletionBlockedAgentClient,
    FlakyAgentClient, FlakyPlanner, GeneratedV2FanoutRunClient, GeneratedV2RunClient,
    GeneratedV2SlowFanoutRunClient, GeneratedV2WorktreeRunClient, GuttedImplementationPlanner,
    InvalidItemsThenRepairAgentClient, InvalidPlanner, PlannerRepairRetryClient,
    SavedV2TemplateRunClient, request, runner, standard_task_file,
};
use super::{LiveApprovalMode, plan_live, run_live_action};
use archon_workflow::stage_retry::transient_live_agent_error;

fn default_generated_workflow_config() -> archon_core::config::GeneratedWorkflowConfig {
    archon_core::config::GeneratedWorkflowConfig::default()
}

#[path = "workflow_live_execution_tests_a.rs"]
mod workflow_live_execution_tests_a;
use workflow_live_execution_tests_a::*;
#[path = "workflow_live_execution_tests_b.rs"]
mod workflow_live_execution_tests_b;
use workflow_live_execution_tests_b::*;
#[path = "workflow_live_execution_tests_c.rs"]
mod workflow_live_execution_tests_c;
use workflow_live_execution_tests_c::*;
#[path = "workflow_live_execution_tests_d.rs"]
mod workflow_live_execution_tests_d;
use workflow_live_execution_tests_d::*;
