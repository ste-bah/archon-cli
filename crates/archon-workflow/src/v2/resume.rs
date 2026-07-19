#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowV2ResumeDecision {
    ReuseCachedResult,
    Execute,
}
