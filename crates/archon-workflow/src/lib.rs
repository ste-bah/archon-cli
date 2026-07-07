//! Provider-neutral dynamic workflow runtime for Archon.

pub mod acceptance;
pub mod approval;
pub mod bundle;
mod cargo_target_env;
pub mod command;
mod command_execution;
// Legacy-only YAML/spec control-plane support. V2 generated workflows must not
// import these modules; see tests/v2_boundary.rs for the enforced boundary.
mod completion_proof;
pub mod config;
pub mod context;
mod context_output;
pub mod control;
pub mod error;
pub mod events;
mod exec_state;
pub mod executor;
mod executor_fanout;
mod executor_output;
pub mod fanout;
// Legacy-only generated spec patchers. V2 harness validation/result contracts
// replace these for generated workflow runs.
mod generated;
pub mod generated_workflow;
mod item_filter;
pub mod learning;
pub mod lifecycle;
mod persistence;
pub mod planner;
pub mod policy;
pub mod provider_tiers;
pub mod quality_gate;
pub mod reducers;
// Legacy-only remediation and required-artifact artifact inference.
mod remediation_inventory;
mod remediation_items;
mod remediation_noop;
mod request;
pub mod run;
pub mod runner;
mod source_context;
pub mod spec;
mod spec_deser;
mod spec_inference;
mod spec_policy;
mod spec_work_units;
mod spec_write_coordination;
pub mod stage;
pub mod store;
pub mod template;
pub mod tui_events;
pub mod v2;
pub mod web_api;
// Legacy-only completion coverage gates. V2 uses typed task coverage directly.
mod work_unit_coverage;
mod work_unit_gate;
mod work_unit_remediation;
pub mod write_coordinator;

pub use acceptance::{AcceptanceOutcome, TargetFingerprints};
pub use approval::{
    WorkflowApprovalDecision, WorkflowApprovalInspection, WorkflowApprovalRecord,
    WorkflowApprovalStore,
};
pub use bundle::{WorkflowBundle, WorkflowBundleManifest, WorkflowBundleOrigin, WorkflowHarness};
pub use command::{CommandAction, WorkflowCommand};
pub use config::WorkflowConfig;
pub use control::{RunControl, RunControlDecision};
pub use error::{WorkflowError, WorkflowResult};
pub use events::{
    CompactProgress, WorkflowEvent, WorkflowEventKind, WorkflowEventLog, contains_forbidden_field,
};
pub use executor::{ExecutionReport, WorkflowExecutor};
pub use generated_workflow::{
    GeneratedWorkflowKind, GeneratedWorkflowLearningContext, WorkflowGeneratedScaffold,
    WorkflowLearningEvent, WorkflowLearningEvidenceRef, workflow_scaffold_hash,
};
pub use learning::{
    Verification, WorkflowLearningRecord, WorkflowLearningSink, WorkflowRunLearningSummary,
    learning_records,
};
pub use lifecycle::{LifecycleAction, LifecycleController, ResumeClassification, classify_resume};
pub use planner::{HeuristicWorkflowPlanner, WorkflowPlanner};
pub use policy::{PolicyDecision, WorkflowPolicy};
pub use provider_tiers::{
    ProviderFamily, ProviderTierResolver, ResolvedProviderTier, classify_provider,
};
pub use quality_gate::{ForcedAcceptAudit, QualityGate, QualityGateDecision};
pub use reducers::{ReducerInput, ReducerOutput, ReducerRegistry};
pub use run::{ArtifactRef, RunStatus, StageStatus, WorkflowRun};
pub use runner::{DeterministicStageRunner, StageRunOutput, StageRunRequest, WorkflowStageRunner};
pub use spec::{
    ArtifactPolicy, ProviderTier, ReducerKind, RetryPolicy, StageKind, StageSpec, WorkflowSpec,
};
pub use store::WorkflowStore;
pub use template::{
    SavedWorkflowCommand, SavedWorkflowTemplate, TemplateRegistry, WorkflowCommandRegistry,
};
pub use v2::{
    BranchFailureKind, PROJECT_ARTIFACT_POLICY_VERSION, WorkflowV2AgentAdapter,
    WorkflowV2AgentClient, WorkflowV2AgentError, WorkflowV2AgentRequest, WorkflowV2Artifact,
    WorkflowV2ArtifactRequirement, WorkflowV2BranchOutcome, WorkflowV2CallExecution,
    WorkflowV2CallRecord, WorkflowV2CancellationToken, WorkflowV2Checkpoint, WorkflowV2CommandKind,
    WorkflowV2CommandRecord, WorkflowV2CommandStatus, WorkflowV2ConvergenceController,
    WorkflowV2ConvergenceDecision, WorkflowV2ConvergenceError, WorkflowV2ConvergenceStatus,
    WorkflowV2Evidence, WorkflowV2EvidenceKind, WorkflowV2FanoutItem, WorkflowV2FanoutReport,
    WorkflowV2FileRecord, WorkflowV2FinalReport, WorkflowV2FinalReportBuilder,
    WorkflowV2FinalReportError, WorkflowV2Harness, WorkflowV2HostCall, WorkflowV2HostMethod,
    WorkflowV2HostOptions, WorkflowV2ImplementationInspector, WorkflowV2ImplementationStatus,
    WorkflowV2InspectionDecision, WorkflowV2InspectionError, WorkflowV2PrdIntake,
    WorkflowV2PrdIntakeError, WorkflowV2ProjectArtifactContext, WorkflowV2RemediationItem,
    WorkflowV2ReportPaths, WorkflowV2ResidualGap, WorkflowV2Result, WorkflowV2ResultStore,
    WorkflowV2ResumeDecision, WorkflowV2RunSummary, WorkflowV2Runtime, WorkflowV2Scheduler,
    WorkflowV2SchedulerConfig, WorkflowV2SourceTargetExpansion, WorkflowV2SourceTaskGraph,
    WorkflowV2SourceTaskItem, WorkflowV2Status, WorkflowV2TaskCompletionEvidence,
    WorkflowV2TaskCompletionEvidenceKind, WorkflowV2TaskCoverage, WorkflowV2TaskCoverageStatus,
    WorkflowV2TaskFileStatus, WorkflowV2TaskRecord, WorkflowV2ValidationError,
    WorkflowV2ValidationResult, WorkflowV2VerificationKind, WorkflowV2VerificationOutcome,
    WorkflowV2VerificationStatus, WorkflowV2WorkItem, WorkflowV2WorkItemKind,
    WorkflowV2WriteAssignment, WorkflowV2WriteConflict, WorkflowV2WriteItem, WorkflowV2WriteMode,
    WorkflowV2WritePlan, WorkflowV2WritePlanner, WorkflowV2WriteSafetyError, WorkflowV2WriteWave,
    has_project_artifact_evidence, normalize_project_artifact_files,
    normalize_target_for_repository, normalize_targets_for_repository,
    project_artifact_context_from_v2_root, test_command, validate_changed_files,
    validate_changed_files_for_repository,
};
pub use write_coordinator::{
    ItemId, ResourceKey, SerialFallbackReason, TargetFilesSource, WaveId, WriteBoundaryProbe,
    WriteCoordinatorConfig, WriteCoordinatorRuntime, WritePlan,
};
