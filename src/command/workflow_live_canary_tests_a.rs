use super::*;

#[path = "workflow_live_canary_usage_tests.rs"]
mod usage_tests;

pub(super) const CANARY_TASK_ID: &str = "TASK-TDL-001";
pub(super) const CANARY_ARTIFACT_REL: &str = ".archon/artifacts/TASK-TDL-001/gap-audit.md";

pub(super) type DecomposedLifecycleEnvGuard =
    crate::command::workflow_live::workflow_live_v2::LifecycleEnvGuard;

/// Scripted stand-in for every agent role in the decomposed-PRD scaffold.
///
/// Responses are keyed on prompt content (the scaffold's `task:` strings), not
/// call order, so the client survives lifecycle refactors. Implementation and
/// remediation agents obey instructions literally: the artifact file is
/// written only when the prompt contains its path. Verification agents check
/// the filesystem like a real focused-verification agent would.
pub(super) struct CanaryAgentClient {
    pub(super) project_root: PathBuf,
    pub(super) prompts: CanaryMutex<Vec<String>>,
}
