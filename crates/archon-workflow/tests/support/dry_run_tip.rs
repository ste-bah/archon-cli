//! The dry runs' host: a runner that builds in its own target directory.
#![allow(dead_code)]
use std::path::{Path, PathBuf};

use archon_workflow::task_universe::WorkflowV2TaskUniverse;
use archon_workflow::*;

/// A host whose runner builds in its own target directory.
pub struct TipDispatch {
    pub target: PathBuf,
}

#[async_trait::async_trait]
impl archon_workflow::agent_dispatch_port::WorkflowAgentDispatch for TipDispatch {
    fn fanout_parallelism(&self, _: Option<usize>) -> usize {
        1
    }
    async fn host_command_env(
        &self,
        _: &Path,
    ) -> archon_workflow::agent_dispatch_port::HostCommandEnv {
        archon_workflow::agent_dispatch_port::HostCommandEnv {
            vars: vec![
                ("CARGO_TARGET_DIR".into(), self.target.display().to_string()),
                ("RUST_MIN_STACK".into(), "8388608".into()),
            ],
            hold: None,
        }
    }
    fn baseline_test_timeout(&self) -> Option<std::time::Duration> {
        Some(std::time::Duration::from_secs(5_400))
    }
    async fn run_call(
        &self,
        _: &str,
        _: Option<String>,
        _: &WorkflowV2CallExecution,
        _: &archon_workflow::v2::WorkflowV2AgentAdapter,
        _: Option<&WorkflowV2ResultStore>,
        _: Option<&WorkflowV2TaskUniverse>,
    ) -> WorkflowResult<WorkflowV2Result> {
        Err(WorkflowError::StageFailed(
            "no agent runs in a dry run".into(),
        ))
    }
}
