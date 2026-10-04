//! Discovery has an idle budget, never a total budget. The cancellation guard
//! detaches bounded delivery so even an unresponsive transport cannot hold up
//! the caller after its deadline.
use super::*;
use futures_util::StreamExt;
use rmcp::ClientHandler;
use rmcp::handler::client::progress::ProgressDispatcher;
use rmcp::model::{ListToolsRequest, ProgressNotificationParam};
use rmcp::service::NotificationContext;

#[derive(Clone, Default)]
pub(super) struct DiscoveryHandler(pub ProgressDispatcher);
impl ClientHandler for DiscoveryHandler {
    async fn on_progress(
        &self,
        params: ProgressNotificationParam,
        _context: NotificationContext<RoleClient>,
    ) {
        self.0.handle_notification(params).await;
    }
}

impl McpClient {
    /// Retrieve tools with a deadline that resets only on increasing progress.
    /// Each concurrent request has its own token and idle clock.
    pub async fn list_tools_with_idle_timeout(
        &self,
        idle: Duration,
    ) -> Result<Vec<McpToolDef>, McpError> {
        let handle = tokio::time::timeout(
            idle,
            self.service.send_cancellable_request(
                ClientRequest::ListToolsRequest(ListToolsRequest::default()),
                PeerRequestOptions::no_options(),
            ),
        )
        .await
        .map_err(|_| McpError::Timeout(idle))?
        .map_err(|error| self.discovery_error(error))?;
        let mut progress = self.progress.subscribe(handle.progress_token.clone()).await;
        let guard =
            crate::call_cancellation::CancelOnDrop::new(handle.peer.clone(), handle.id.clone());
        let mut response = handle.rx;
        let sleep = tokio::time::sleep(idle);
        tokio::pin!(sleep);
        let mut last_progress = None;
        let result = loop {
            tokio::select! {
                // Completed responses win over a simultaneous timeout.
                biased;
                result = &mut response => {
                    guard.disarm();
                    break result.unwrap_or(Err(rmcp::service::ServiceError::TransportClosed))
                        .map_err(|error| self.discovery_error(error))?;
                }
                _ = &mut sleep => return Err(McpError::Timeout(idle)),
                Some(update) = progress.next() => {
                    if update.progress.is_finite() && last_progress.is_none_or(|last| update.progress > last) {
                        last_progress = Some(update.progress);
                        sleep.as_mut().reset(tokio::time::Instant::now() + idle);
                    }
                }
            }
        };
        let ServerResult::ListToolsResult(result) = result else {
            return Err(self.discovery_error("wrong result type"));
        };
        Ok(result
            .tools
            .into_iter()
            .map(|tool| McpToolDef {
                name: tool.name.to_string(),
                description: tool.description.map(|description| description.to_string()),
                input_schema: serde_json::Value::Object((*tool.input_schema).clone()),
                annotations: tool.annotations,
                meta: tool.meta.and_then(|meta| serde_json::to_value(meta).ok()),
                server_name: self.server_name.clone(),
            })
            .collect())
    }

    fn discovery_error(&self, error: impl std::fmt::Display) -> McpError {
        McpError::ToolCallFailed(format!(
            "tools/list failed on '{}': {error}",
            self.server_name
        ))
        .redacted()
    }
}
