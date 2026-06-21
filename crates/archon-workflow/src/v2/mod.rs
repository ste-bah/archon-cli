//! Claude-style dynamic workflow runtime boundary.
//!
//! V2 keeps generated workflow harness orchestration separate from the legacy
//! YAML-stage executor. The concrete runtime lands in the follow-on PRD-017
//! tasks; this module establishes the public boundary used by those slices.

pub mod agent_adapter;
pub mod harness;
mod harness_safety;
pub mod host_api;
pub mod implementation_inspection;
pub mod prd_intake;
pub mod remediation;
pub mod report;
pub mod result;
pub mod result_store;
pub mod resume;
pub mod runtime;
pub mod scheduler;
pub mod task_record;
pub mod validation;
pub mod write_mode;

pub use agent_adapter::{
    WorkflowV2AgentAdapter, WorkflowV2AgentClient, WorkflowV2AgentError, WorkflowV2AgentRequest,
};
pub use harness::{WorkflowV2HarnessError, WorkflowV2HarnessPlan, WorkflowV2HarnessValidator};
pub use host_api::{
    WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2HostOptions, WorkflowV2WriteMode,
};
pub use implementation_inspection::{
    WorkflowV2ImplementationInspector, WorkflowV2InspectionDecision, WorkflowV2InspectionError,
    WorkflowV2WorkItem, WorkflowV2WorkItemKind,
};
pub use prd_intake::{WorkflowV2PrdIntake, WorkflowV2PrdIntakeError};
pub use remediation::{
    WorkflowV2ConvergenceController, WorkflowV2ConvergenceDecision, WorkflowV2ConvergenceError,
    WorkflowV2ConvergenceStatus, WorkflowV2RemediationItem, WorkflowV2VerificationKind,
    WorkflowV2VerificationOutcome, WorkflowV2VerificationStatus, test_command,
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
pub use result_store::{WorkflowV2CallRecord, WorkflowV2Checkpoint, WorkflowV2ResultStore};
pub use resume::WorkflowV2ResumeDecision;
pub use runtime::{WorkflowV2CallExecution, WorkflowV2RunSummary, WorkflowV2Runtime};
pub use scheduler::{
    WorkflowV2BranchOutcome, WorkflowV2CancellationToken, WorkflowV2FanoutItem,
    WorkflowV2FanoutReport, WorkflowV2Scheduler, WorkflowV2SchedulerConfig,
};
pub use task_record::{
    WorkflowV2ImplementationStatus, WorkflowV2TaskFileStatus, WorkflowV2TaskRecord,
};
pub use validation::{WorkflowV2ValidationError, WorkflowV2ValidationResult};
pub use write_mode::{
    WorkflowV2WriteAssignment, WorkflowV2WriteConflict, WorkflowV2WriteItem, WorkflowV2WritePlan,
    WorkflowV2WritePlanner, WorkflowV2WriteSafetyError, WorkflowV2WriteWave,
    validate_changed_files,
};

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
