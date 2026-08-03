//! Host side of `archon_workflow::llm_client_port`.
//!
//! `archon-workflow` cannot reach `archon-pipeline` (see the port's module doc),
//! so the concrete subagent-capable client is built here and injected. This type
//! is the whole of the wiring: it holds the config and environment the CLI
//! already read, and defers everything else to the existing builder.

use std::sync::Arc;

use archon_core::config::ArchonConfig;
use archon_core::env_vars::ArchonEnvVars;
use archon_pipeline::runner::LlmClient;
use archon_workflow::error::{WorkflowError, WorkflowResult};
use archon_workflow::llm_client_port::{WorkflowLlmClientFactory, WorkflowLlmClientRequest};
use async_trait::async_trait;

use crate::command::pipeline_support::build_subagent_pipeline_adapter;

/// Owns its config rather than borrowing it, so the type carries no lifetime
/// parameter. `#[async_trait]` and lifetime-parametrised implementors interact
/// badly, and a factory outliving the borrow it was built from is the shape
/// Wave B will want anyway. The cost is one clone per CLI invocation, against a
/// run that is about to make network calls.
pub(crate) struct SubagentPipelineClientFactory {
    config: ArchonConfig,
    env_vars: ArchonEnvVars,
}

impl SubagentPipelineClientFactory {
    pub(crate) fn new(config: &ArchonConfig, env_vars: &ArchonEnvVars) -> Self {
        Self {
            config: config.clone(),
            env_vars: env_vars.clone(),
        }
    }
}

#[async_trait(?Send)]
impl WorkflowLlmClientFactory<dyn LlmClient> for SubagentPipelineClientFactory {
    async fn build_client(
        &self,
        request: WorkflowLlmClientRequest,
    ) -> WorkflowResult<Arc<dyn LlmClient>> {
        build_subagent_pipeline_adapter(
            &self.config,
            &self.env_vars,
            &request.origin,
            &request.cwd,
            &request.session_id,
        )
        .await
        .map_err(WorkflowError::port)
    }
}
