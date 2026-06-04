use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::WorkflowResult;
use crate::spec::{ProviderTier, StageKind};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StageRunRequest {
    pub run_id: String,
    pub stage_id: String,
    pub stage_kind: StageKind,
    pub agent: Option<String>,
    pub task: String,
    pub attempt: u32,
    pub provider_tier: ProviderTier,
    pub depends_on: Vec<String>,
    pub input: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StageRunOutput {
    pub body: String,
    pub extension: String,
    pub provider_id: Option<String>,
    pub resolved_model: Option<String>,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub cost_usd: f64,
}

impl StageRunOutput {
    pub fn markdown(body: impl Into<String>) -> Self {
        Self {
            body: body.into(),
            extension: "md".into(),
            provider_id: None,
            resolved_model: None,
            tokens_in: 0,
            tokens_out: 0,
            cost_usd: 0.0,
        }
    }
}

#[async_trait]
pub trait WorkflowStageRunner: Send + Sync {
    async fn run_stage(&self, request: StageRunRequest) -> WorkflowResult<StageRunOutput>;

    /// Maximum number of stages this runner can execute concurrently, when the
    /// runner is backed by a bounded resource (e.g. a subagent manager with a
    /// hard concurrency cap that *rejects* — rather than queues — overflow).
    ///
    /// Fan-out scheduling clamps its semaphore width to this value so the
    /// number of in-flight items never exceeds what the runner can accept.
    /// Returning `None` (the default) means "no runner-imposed limit"; only the
    /// spec/policy `max_parallelism` applies.
    fn max_concurrency(&self) -> Option<usize> {
        None
    }
}

#[derive(Debug, Default)]
pub struct DeterministicStageRunner;

#[async_trait]
impl WorkflowStageRunner for DeterministicStageRunner {
    async fn run_stage(&self, request: StageRunRequest) -> WorkflowResult<StageRunOutput> {
        let agent = request.agent.as_deref().unwrap_or("none");
        Ok(StageRunOutput::markdown(format!(
            "# Stage {}\n\nKind: `{:?}`\nAgent: `{}`\n",
            request.stage_id, request.stage_kind, agent
        )))
    }
}
