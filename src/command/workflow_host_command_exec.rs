//! Injectable persisted HostCommand execution boundary.

use archon_workflow::{HostCommandRequest, HostCommandResult, WorkflowResult};
use async_trait::async_trait;

#[async_trait]
pub(crate) trait WorkflowHostCommandExecutor: Send + Sync {
    fn call_identity(&self, request: &HostCommandRequest) -> WorkflowResult<String>;

    async fn execute(&self, request: HostCommandRequest) -> WorkflowResult<HostCommandResult>;
}
