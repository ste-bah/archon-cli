//! Provider-neutral dynamic workflow runtime for Archon.

pub mod acceptance;
pub mod agent_dispatch_port;
pub mod agent_select;
pub mod approval;
pub mod bundle;
pub mod command;
mod command_execution;
mod completion_proof;
pub mod config;
pub mod context;
mod context_output;
pub mod control;
pub mod error;
pub mod events;
mod executor_output;
pub mod fanout;
pub mod generated_contract;
pub mod generated_lifecycle_remediation;
pub mod generated_lifecycle_support;
pub mod generated_workflow;
mod item_filter;
pub mod learning;
pub mod lifecycle;
pub mod lifecycle_host_port;
pub mod llm_client_port;
pub mod llm_retry;
pub mod lower_workflow;
mod persistence;
pub mod planner;
pub mod policy;
pub mod provider_tiers;
mod remediation_items;
pub mod repo_root;
mod request;
pub mod run;
pub mod runner;
mod source_context;
pub mod spec;
mod spec_compat;
mod spec_deser;
mod spec_inference;
mod spec_policy;
mod spec_work_units;
pub mod stage;
pub mod stage_activity;
pub mod stage_command_policy;
pub mod stage_item_output;
pub mod stage_prompt;
pub mod store;
pub mod task_universe;
pub mod template;
pub mod tool_declarations;
pub mod tui_events;
pub mod ui_sink_port;
pub mod v2;
pub mod web_api;
mod work_unit_coverage;
mod work_unit_gate;
pub mod write_coordinator;

pub use acceptance::{AcceptanceOutcome, TargetFingerprints};
pub use agent_dispatch_port::WorkflowAgentDispatch;
pub use approval::{
    WorkflowApprovalDecision, WorkflowApprovalInspection, WorkflowApprovalRecord,
    WorkflowApprovalStore,
};
pub use bundle::{WorkflowBundle, WorkflowBundleManifest, WorkflowBundleOrigin, WorkflowHarness};
pub use command::{CommandAction, WorkflowCommand};
pub use config::WorkflowConfig;
pub use control::{RunControl, RunControlDecision, poll_v2_run_control};
pub use error::{WorkflowError, WorkflowResult};
pub use events::{
    CompactProgress, WorkflowEvent, WorkflowEventKind, WorkflowEventLog, contains_forbidden_field,
};
pub use generated_workflow::{
    GeneratedWorkflowKind, GeneratedWorkflowLearningContext, WorkflowGeneratedScaffold,
    WorkflowLearningEvent, WorkflowLearningEvidenceRef, workflow_scaffold_hash,
};
pub use learning::{
    LEARNING_RECORDS_FILE, Verification, WorkflowLearningRecord, WorkflowLearningSink,
    WorkflowRunLearningSummary, learning_records, learning_records_path, read_learning_records,
};
pub use lifecycle::{LifecycleAction, LifecycleController, ResumeClassification, classify_resume};
pub use lifecycle_host_port::{LifecycleHost, TERMINAL_HOST_CALL_MARKER};
pub use llm_client_port::{
    WorkflowAgentCall, WorkflowAgentOutcome, WorkflowAgentSpec, WorkflowAgentToolAccess,
    WorkflowAgentToolUse, WorkflowLlmClient, WorkflowLlmClientFactory, WorkflowLlmClientRequest,
    WorkflowProviderEnv,
};
pub use lower_workflow::lower_workflow_spec;
pub use planner::{HeuristicWorkflowPlanner, WorkflowPlanner};
pub use policy::{PolicyDecision, WorkflowPolicy};
pub use provider_tiers::{
    ProviderFamily, ProviderTierResolver, ResolvedProviderTier, classify_provider,
};
pub use run::{ArtifactRef, RunStatus, StageStatus, WorkflowRun};
pub use runner::{StageRunOutput, StageRunRequest, WorkflowStageRunner};
pub use spec::{ProviderTier, ReducerKind, RetryPolicy, StageKind, StageSpec, WorkflowSpec};
pub use store::WorkflowStore;
pub use template::{
    SavedWorkflowCommand, SavedWorkflowTemplate, TemplateRegistry, WorkflowCommandRegistry,
};
pub use ui_sink_port::{
    SharedWorkflowUiSink, WorkflowActivityStatus, WorkflowActivityUpdate, WorkflowUiDeliveryError,
    WorkflowUiEvent, WorkflowUiResult, WorkflowUiSink,
};
pub use v2::{
    BranchFailureKind, PROJECT_ARTIFACT_POLICY_VERSION, WorkflowV2AgentAdapter,
    WorkflowV2AgentClient, WorkflowV2AgentError, WorkflowV2AgentRequest, WorkflowV2Artifact,
    WorkflowV2ArtifactRequirement, WorkflowV2BranchOutcome, WorkflowV2CallExecution,
    WorkflowV2CallRecord, WorkflowV2CancellationToken, WorkflowV2Checkpoint, WorkflowV2CommandKind,
    WorkflowV2CommandRecord, WorkflowV2CommandStatus, WorkflowV2Evidence, WorkflowV2EvidenceKind,
    WorkflowV2FanoutItem, WorkflowV2FanoutReport, WorkflowV2FileRecord, WorkflowV2FinalReport,
    WorkflowV2FinalReportBuilder, WorkflowV2FinalReportError, WorkflowV2Harness,
    WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2HostOptions,
    WorkflowV2ImplementationInspector, WorkflowV2ImplementationStatus,
    WorkflowV2InspectionDecision, WorkflowV2InspectionError, WorkflowV2PrdIntake,
    WorkflowV2PrdIntakeError, WorkflowV2ProjectArtifactContext, WorkflowV2RejectedOutput,
    WorkflowV2ReportPaths, WorkflowV2ResidualGap, WorkflowV2Result, WorkflowV2ResultStore,
    WorkflowV2Scheduler, WorkflowV2SchedulerConfig, WorkflowV2SourceTargetExpansion,
    WorkflowV2SourceTaskGraph, WorkflowV2SourceTaskItem, WorkflowV2Status,
    WorkflowV2TaskCompletionEvidence, WorkflowV2TaskCompletionEvidenceKind, WorkflowV2TaskCoverage,
    WorkflowV2TaskCoverageStatus, WorkflowV2TaskFileStatus, WorkflowV2TaskInvalidation,
    WorkflowV2TaskRecord, WorkflowV2ValidationError, WorkflowV2ValidationResult,
    WorkflowV2WorkItem, WorkflowV2WorkItemKind, WorkflowV2WriteAssignment, WorkflowV2WriteConflict,
    WorkflowV2WriteItem, WorkflowV2WriteMode, WorkflowV2WritePlan, WorkflowV2WritePlanner,
    WorkflowV2WriteSafetyError, WorkflowV2WriteWave, has_project_artifact_evidence,
    normalize_project_artifact_files, normalize_target_for_repository,
    normalize_targets_for_repository, project_artifact_context_from_v2_root, stable_value_hash,
    validate_changed_files, validate_changed_files_for_repository,
};
pub use write_coordinator::{
    ItemId, ResourceKey, SerialFallbackReason, TargetFilesSource, WaveId, WriteBoundaryProbe,
    WriteCoordinatorConfig, WriteCoordinatorRuntime, WritePlan,
};
