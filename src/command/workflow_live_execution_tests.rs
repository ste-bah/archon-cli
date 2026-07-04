use std::collections::BTreeSet;
use std::process::Command;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use archon_workflow::{
    CommandAction, GeneratedWorkflowKind, LifecycleAction, LifecycleController, ProviderTier,
    RunStatus, StageKind, StageRunRequest, WorkflowApprovalStore, WorkflowBundle,
    WorkflowBundleOrigin, WorkflowCommandRegistry, WorkflowLearningEvent,
    WorkflowLearningEvidenceRef, WorkflowSpec, WorkflowStageRunner, WorkflowStore,
    WorkflowV2HarnessValidator, WorkflowV2HostCall, WorkflowV2Status, workflow_scaffold_hash,
};
use serde_json::json;

use super::workflow_live_retry::transient_live_agent_error;
use super::workflow_live_test_support::{
    AlwaysInvalidItemsAgentClient, FlakyAgentClient, FlakyPlanner, GeneratedV2FanoutRunClient,
    GeneratedV2RunClient, GeneratedV2SlowFanoutRunClient, GeneratedV2WorktreeRunClient,
    GuttedImplementationPlanner, InvalidItemsThenRepairAgentClient, InvalidPlanner,
    SavedV2TemplateRunClient, request, runner,
};
use super::{LiveApprovalMode, plan_live, run_live_action};

fn default_generated_workflow_config() -> archon_core::config::GeneratedWorkflowConfig {
    archon_core::config::GeneratedWorkflowConfig::default()
}

include!("workflow_live_execution_tests_a.rs");
include!("workflow_live_execution_tests_b.rs");
include!("workflow_live_execution_tests_c.rs");
include!("workflow_live_execution_tests_d.rs");
