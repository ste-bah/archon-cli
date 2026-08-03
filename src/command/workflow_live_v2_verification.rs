use archon_workflow::{
    BranchFailureKind, WorkflowV2BranchOutcome, WorkflowV2CommandStatus, WorkflowV2Evidence,
    WorkflowV2EvidenceKind, WorkflowV2Result, WorkflowV2Status, WorkflowV2TaskCoverageStatus,
};

#[path = "workflow_live_v2_verification_a.rs"]
mod workflow_live_v2_verification_a;
pub(crate) use workflow_live_v2_verification_a::*;
#[path = "workflow_live_v2_verification_b.rs"]
mod workflow_live_v2_verification_b;
pub(super) use workflow_live_v2_verification_b::*;
