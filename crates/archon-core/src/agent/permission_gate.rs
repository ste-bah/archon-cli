use archon_permissions::checker::PermissionChecker;
use archon_permissions::mode::{PermissionDecision, PermissionMode};

use super::*;

impl Agent {
    /// Shared handle to the live permission mode.
    ///
    /// The mode is read on every tool call and written by `/permissions`, so
    /// hosts that expose it (the TUI slash command, the IDE's
    /// `archon/config`) need the same `Arc` the agent reads rather than a
    /// copy that would drift out of agreement with the gate.
    pub fn permission_mode_handle(&self) -> Arc<Mutex<String>> {
        Arc::clone(&self.config.permission_mode)
    }

    pub(super) fn permission_checker_decision(
        &self,
        raw_mode: &str,
        tool_name: &str,
        tool_args: &str,
        description: &str,
    ) -> PermissionDecision {
        let mode = raw_mode.parse::<PermissionMode>().unwrap_or_default();
        PermissionChecker::new(mode, self.config.permission_rules.clone()).check(
            tool_name,
            description,
            tool_args,
        )
    }

    pub(super) async fn request_tool_permission(
        &self,
        tool: &PendingToolCall,
        mode: &str,
        description: String,
    ) -> bool {
        let perm_agg = self
            .fire_hook(
                crate::hooks::HookEvent::PermissionRequest,
                serde_json::json!({
                    "hook_event": "PermissionRequest",
                    "tool_name": tool.name,
                    "mode": mode,
                }),
            )
            .await;
        self.apply_permission_updates_from_hooks(&perm_agg);

        self.send_event(AgentEvent::PermissionRequired {
            tool: tool.name.clone(),
            description,
        })
        .await;

        if let Some(ref rx) = self.permission_response_rx {
            let mut rx = rx.lock().await;
            match tokio::time::timeout(std::time::Duration::from_secs(120), rx.recv()).await {
                Ok(Some(true)) => {
                    self.send_event(AgentEvent::PermissionGranted {
                        tool: tool.name.clone(),
                    })
                    .await;
                    tracing::info!(tool = %tool.name, mode = %mode, "permission approved");
                    true
                }
                _ => {
                    self.fire_permission_denied_hook(tool, mode, "user_denied_or_timeout")
                        .await;
                    tracing::info!(
                        tool = %tool.name,
                        mode = %mode,
                        "permission denied or timed out"
                    );
                    false
                }
            }
        } else {
            tracing::info!(
                tool = %tool.name,
                mode = %mode,
                "no permission channel, auto-approved"
            );
            true
        }
    }

    pub(super) async fn fire_permission_denied_hook(
        &self,
        tool: &PendingToolCall,
        mode: &str,
        reason: &str,
    ) {
        self.fire_hook(
            crate::hooks::HookEvent::PermissionDenied,
            serde_json::json!({
                "hook_event": "PermissionDenied",
                "tool_name": tool.name,
                "mode": mode,
                "reason": reason,
            }),
        )
        .await;
        self.send_event(AgentEvent::PermissionDenied {
            tool: tool.name.clone(),
            reason: Some(reason.to_string()),
        })
        .await;
    }

    pub(super) fn apply_permission_updates_from_hooks(
        &self,
        perm_agg: &crate::hooks::AggregatedHookResult,
    ) {
        if perm_agg.updated_permissions.is_empty() {
            return;
        }
        let authority = crate::hooks::SourceAuthority::Project;
        let errors = crate::hooks::apply_permission_updates(
            &perm_agg.updated_permissions,
            &authority,
            self.permission_store.as_ref(),
        );
        for err in &errors {
            tracing::error!("permission update failed: {}", err);
        }
    }
}

#[cfg(test)]
#[path = "permission_gate_tests.rs"]
mod tests;
