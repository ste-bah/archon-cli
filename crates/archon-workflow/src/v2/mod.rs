//! Claude-style dynamic workflow runtime boundary.
//!
//! V2 keeps generated workflow harness orchestration separate from the legacy
//! YAML-stage executor. The concrete runtime lands in the follow-on PRD-017
//! tasks; this module establishes the public boundary used by those slices.

pub mod agent_adapter;
mod agent_output_fault;
mod agent_output_normalize;
mod agent_prompt;
mod agent_repair;
pub mod artifact_emptiness;
pub mod artifact_path_guard;
pub mod artifact_presence;
pub mod artifact_refs;
pub mod branch_cache;
pub mod branch_evidence;
pub mod branch_stamping;
pub mod call_data;
pub mod call_execution;
pub mod completion_credit;
pub mod completion_evidence;
pub(crate) mod contract_code_targets;
pub mod declarative_floor;
mod declarative_floor_collect;
pub(crate) mod declared_output_contract;
pub mod decomposed_prd_plan;
pub mod decomposition;
pub mod deliverable_contract;
pub mod finalization;
pub mod gate_envelope;
pub mod host_api;
pub mod host_command;
pub mod implementation_inspection;
pub(crate) mod inventory_artifact_seeding;
pub mod lifecycle_driver;
pub mod lifecycle_policy;
pub mod lifecycle_prompts;
pub mod local_host;
pub mod manifest_scope;
pub mod orchestrator_actions;
pub mod outcome_envelope;
pub mod plan_metadata;
pub mod prd_intake;
pub(crate) mod project_artifact_admission;
mod project_artifact_completion;
pub mod project_artifact_contract;
pub(crate) mod project_artifact_contract_roots;
mod project_artifact_prompt;
pub mod project_artifact_results;
pub mod project_artifact_stamping;
pub mod project_artifact_write_roots;
pub mod project_artifacts;
pub mod publication;
pub mod report;
pub mod restart;
pub mod result;
pub mod result_store;
pub mod review_findings;
pub mod run_state_sync;
pub mod scheduler;
pub mod script;
/// Host-side semantic-preservation enforcement for LLM repair adoption. Only
/// the lifecycle driver consults it.
pub(crate) mod semantic_preservation;
pub mod source_graph;
pub mod source_pack;
pub mod target_expansion;
pub mod task_record;
pub mod transport_retry;
pub mod validation;
pub mod verification;
pub mod write;
pub mod write_claim_gate;
pub mod write_mode;
mod write_mode_paths;
pub mod write_scope_extension;
#[cfg(test)]
#[path = "write_scope_extension_wiring_tests.rs"]
mod write_scope_extension_wiring_tests;

pub use agent_adapter::{
    WorkflowV2AgentAdapter, WorkflowV2AgentClient, WorkflowV2AgentError, WorkflowV2AgentRequest,
};
pub use call_execution::WorkflowV2CallExecution;
pub use declarative_floor::{
    DeclarativeFloorEvaluation, DeclarativeFloorFacts, declarative_floor_deferral_reason,
    evaluate_declarative_floor,
};
pub use declarative_floor_collect::collect_declarative_floor_facts;
pub use decomposition::{
    DecompositionAttemptStateV1, DecompositionPhase, FIXED_DECOMPOSITION_STATE_SCHEMA_VERSION,
    FIXED_DECOMPOSITION_TEMPLATE_VERSION, FixedDecompositionStateV1, FixedRunIdentityV1,
    SubjectDisposition, WorkflowRunKind, verify_fixed_resume_identity,
};
pub use finalization::{
    FINALIZATION_RECORD_SCHEMA_VERSION, FinalizationRecordV1, ObserverAuthority,
    PortableAcceptanceIdentityV1, RUN_END_OBSERVER_EXPECTED_ARTIFACT_PATHS,
    RUN_END_OBSERVER_SNAPSHOT_SCHEMA_VERSION, RunEndAcceptanceObserverSnapshotV1,
    RunEndObserverOutcomeV1, RunEndObserverStateV1, observer_eligible,
};
pub use gate_envelope::{
    GATE_ENVELOPE_SCHEMA_VERSION, GateEnvelopeV1, GateOperationalError, GatePolicyFinding,
    RemediationScope,
};
pub use host_api::{
    AgentResultMode, WorkflowV2ArtifactRequirement, WorkflowV2HostCall, WorkflowV2HostMethod,
    WorkflowV2HostOptions, WorkflowV2WriteMode,
};
pub use host_command::{
    CommandCapability, CommandCapabilityCatalog, CommandPostconditionEvaluation,
    EnvironmentProfileId, HostCommandRequest, HostCommandResult, HostCommandSubject, StdinDelivery,
    host_command_call_id,
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
pub use publication::{
    PREPARED_PUBLICATION_SCHEMA_VERSION, PUBLICATION_RECEIPT_SCHEMA_VERSION,
    PreparedPublicationEntry, PreparedPublicationV1, PublicationReceiptV1,
    PublishedArtifactReceipt,
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
