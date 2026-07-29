use thiserror::Error;

use super::WorkflowV2Result;
use super::agent_adapter::{WorkflowV2AgentAdapter, WorkflowV2AgentClient, WorkflowV2AgentRequest};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RepairErrorClass {
    Contract,
    Ownership,
    Execution,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum WorkflowV2AgentError {
    #[error("{0}")]
    MalformedOutput(String),
    #[error("{0}")]
    InvalidResult(String),
    #[error("agent output contains restored-context summary text")]
    RestoredContextSummary,
    #[error("agent output contains a confirmation question instead of executing")]
    ConfirmationQuestion,
    #[error(
        "implementation agent returned a plan-only result instead of edits or typed no-op proof"
    )]
    PlanOnlyImplementation,
    #[error(
        "implementation agent returned accepted status without changed files; use noop with task coverage evidence when no edits are required"
    )]
    ImplementationAcceptedWithoutChanges,
    #[error("implementation noop requires typed task_coverage evidence")]
    ImplementationNoopWithoutTaskCoverage,
    #[error(
        "task declares required_tools; a no-op is not acceptable — exercise the declared tools and return accepted with fresh command/artifact evidence, or block honestly"
    )]
    ImplementationNoopWithDeclaredRequiredTools,
    /// Carries EVERY unexercised tool, not the first. One-at-a-time reporting
    /// spends an attempt per missing tool, and the repair budget cannot fund
    /// that: two missing-tool errors share the `Contract` repair class, so the
    /// second can never earn an extra attempt from `differs_from`.
    #[error(
        "task declares required_tools; an accepted result must show an actual invocation of every declared tool (a captured success OR a captured failure counts), but these required tools were never exercised this run: {}. Run every one of them and return the captured results, or block honestly with the captured failures. Do not assert a tool is unavailable without attempting it.",
        .0.join(", ")
    )]
    ImplementationAcceptedWithRequiredToolUnexercised(Vec<String>),
    #[error(
        "implementation noop with declared project artifacts requires existing artifact evidence"
    )]
    ImplementationNoopMissingProjectArtifactEvidence,
    #[error("implementation agent changed files outside declared target_files: {0}")]
    ImplementationChangedFilesOutsideOwnership(String),
    #[error("read-only agent result must not claim changed files")]
    ReadOnlyChangedFiles,
    #[error("agent transport failed: {0}")]
    Transport(String),
    #[error("schema repair failed after bounded retries: root={first_error}; last={repair_error}")]
    RepairExhausted {
        first_error: Box<WorkflowV2AgentError>,
        repair_error: Box<WorkflowV2AgentError>,
    },
}

impl WorkflowV2AgentError {
    pub fn differs_from(&self, other: &Self) -> bool {
        self.repair_class() != other.repair_class()
    }

    fn repair_class(&self) -> RepairErrorClass {
        match self {
            Self::ImplementationChangedFilesOutsideOwnership(_) | Self::ReadOnlyChangedFiles => {
                RepairErrorClass::Ownership
            }
            Self::Transport(_) => RepairErrorClass::Execution,
            _ => RepairErrorClass::Contract,
        }
    }
}

impl WorkflowV2AgentAdapter {
    pub async fn run_with_repair<C>(
        &self,
        client: &C,
        request: &WorkflowV2AgentRequest,
    ) -> Result<WorkflowV2Result, WorkflowV2AgentError>
    where
        C: WorkflowV2AgentClient + Sync,
    {
        let first = client
            .run_agent_request(request, self.build_prompt(request))
            .await?;
        let first_error = match self.parse_agent_output(request, &first) {
            Ok(result) => return Ok(result),
            Err(error) => error,
        };
        let repaired = match self
            .request_repair(client, request, &first, &first_error)
            .await
        {
            Ok(output) => output,
            Err(last) => return Err(repair_exhausted(first_error, last)),
        };
        let repair_error = match self.parse_agent_output(request, &repaired) {
            Ok(result) => return Ok(result),
            Err(error) => error,
        };
        if repair_error.differs_from(&first_error) {
            let second = match self
                .request_repair(client, request, &repaired, &repair_error)
                .await
            {
                Ok(output) => output,
                Err(last) => return Err(repair_exhausted(first_error, last)),
            };
            return self
                .parse_agent_output(request, &second)
                .map_err(|last| repair_exhausted(first_error, last));
        }
        Err(repair_exhausted(first_error, repair_error))
    }

    async fn request_repair<C>(
        &self,
        client: &C,
        request: &WorkflowV2AgentRequest,
        output: &str,
        error: &WorkflowV2AgentError,
    ) -> Result<String, WorkflowV2AgentError>
    where
        C: WorkflowV2AgentClient + Sync,
    {
        client
            .run_agent_request(request, self.build_repair_prompt(request, output, error))
            .await
    }
}

fn repair_exhausted(
    first_error: WorkflowV2AgentError,
    last_error: WorkflowV2AgentError,
) -> WorkflowV2AgentError {
    WorkflowV2AgentError::RepairExhausted {
        first_error: Box::new(first_error),
        repair_error: Box::new(last_error),
    }
}
