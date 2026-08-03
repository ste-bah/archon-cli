//! Claude-style dynamic workflow runtime boundary.
//!
//! V2 keeps generated workflow harness orchestration separate from the legacy
//! YAML-stage executor. The concrete runtime lands in the follow-on PRD-017
//! tasks; this module establishes the public boundary used by those slices.

pub mod agent_adapter;
mod agent_output_normalize;
mod agent_prompt;
mod agent_repair;
pub mod artifact_refs;
mod branch_evidence;
pub mod branch_stamping;
pub mod call_execution;
pub mod completion_credit;
pub mod decomposed_prd_plan;
pub mod deliverable_contract;
pub mod host_api;
pub mod implementation_inspection;
pub mod lifecycle_driver;
pub mod lifecycle_policy;
pub mod lifecycle_prompts;
pub mod local_host;
pub mod manifest_scope;
pub mod orchestrator_actions;
pub mod outcome_envelope;
pub mod prd_intake;
mod project_artifact_completion;
pub mod project_artifact_contract;
mod project_artifact_prompt;
pub mod project_artifact_results;
pub mod project_artifact_stamping;
pub mod project_artifacts;
pub mod report;
pub mod result;
pub mod result_store;
pub mod scheduler;
/// Host-side semantic-preservation enforcement for LLM repair adoption. Only
/// the lifecycle driver consults it.
pub(crate) mod semantic_preservation;
pub mod source_graph;
pub mod target_expansion;
pub mod task_record;
pub mod validation;
pub mod write_mode;
mod write_mode_paths;

pub use agent_adapter::{
    WorkflowV2AgentAdapter, WorkflowV2AgentClient, WorkflowV2AgentError, WorkflowV2AgentRequest,
};
pub use call_execution::WorkflowV2CallExecution;
pub use host_api::{
    WorkflowV2ArtifactRequirement, WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2HostOptions,
    WorkflowV2WriteMode,
};
pub use implementation_inspection::{
    WorkflowV2ImplementationInspector, WorkflowV2InspectionDecision, WorkflowV2InspectionError,
    WorkflowV2WorkItem, WorkflowV2WorkItemKind,
};
pub use prd_intake::{WorkflowV2PrdIntake, WorkflowV2PrdIntakeError};
pub use project_artifact_results::load_project_artifact_branch_result;
pub use project_artifacts::{
    PROJECT_ARTIFACT_POLICY_VERSION, WorkflowV2ProjectArtifactContext,
    has_project_artifact_evidence, has_project_artifact_requirement,
    normalize_project_artifact_files, project_artifact_context_from_v2_root,
};
pub use report::{
    WorkflowV2FinalReport, WorkflowV2FinalReportBuilder, WorkflowV2FinalReportError,
    WorkflowV2ReportPaths,
};
pub use result::{
    WorkflowV2Artifact, WorkflowV2CommandKind, WorkflowV2CommandRecord, WorkflowV2CommandStatus,
    WorkflowV2Evidence, WorkflowV2EvidenceKind, WorkflowV2FileRecord, WorkflowV2ResidualGap,
    WorkflowV2Result, WorkflowV2Status, WorkflowV2TaskCoverage, WorkflowV2TaskCoverageStatus,
};
pub use result_store::{
    WorkflowV2CallRecord, WorkflowV2Checkpoint, WorkflowV2DeletedBranchOutcome,
    WorkflowV2RejectedOutput, WorkflowV2ResultStore, WorkflowV2SourceTargetExpansion,
    WorkflowV2SourceTaskGraph, WorkflowV2SourceTaskItem, WorkflowV2TaskCompletionEvidence,
    WorkflowV2TaskCompletionEvidenceKind, WorkflowV2TaskInvalidation,
};
pub use scheduler::{
    BranchFailureKind, WorkflowV2BranchOutcome, WorkflowV2CancellationToken, WorkflowV2FanoutItem,
    WorkflowV2FanoutReport, WorkflowV2Scheduler, WorkflowV2SchedulerConfig, stable_value_hash,
};
pub use task_record::{
    WorkflowV2ImplementationStatus, WorkflowV2TaskFileStatus, WorkflowV2TaskRecord,
};
pub use validation::{WorkflowV2ValidationError, WorkflowV2ValidationResult};
pub use write_mode::{
    WorkflowV2WriteAssignment, WorkflowV2WriteConflict, WorkflowV2WriteItem, WorkflowV2WritePlan,
    WorkflowV2WritePlanner, WorkflowV2WriteSafetyError, WorkflowV2WriteWave,
    validate_changed_files, validate_changed_files_for_repository,
};
pub use write_mode_paths::{normalize_target_for_repository, normalize_targets_for_repository};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowV2Harness {
    pub source: String,
}

impl WorkflowV2Harness {
    pub fn new(source: impl Into<String>) -> Self {
        Self {
            source: source.into(),
        }
    }
}
