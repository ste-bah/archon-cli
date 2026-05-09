use archon_permissions::checker::PermissionChecker;
use archon_permissions::mode::{PermissionDecision, PermissionMode};

use super::*;

impl Agent {
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
mod tests {
    use super::*;
    use archon_llm::provider::{LlmError, LlmResponse, ModelInfo, ProviderFeature};
    use archon_permissions::rules::{RuleSet, ToolRule};

    struct MockLlmProvider;

    #[async_trait::async_trait]
    impl LlmProvider for MockLlmProvider {
        fn name(&self) -> &str {
            "mock"
        }

        fn models(&self) -> Vec<ModelInfo> {
            vec![]
        }

        fn supports_feature(&self, _: ProviderFeature) -> bool {
            false
        }

        async fn stream(
            &self,
            _request: LlmRequest,
        ) -> Result<tokio::sync::mpsc::Receiver<StreamEvent>, LlmError> {
            let (_tx, rx) = tokio::sync::mpsc::channel(1);
            Ok(rx)
        }

        async fn complete(&self, _request: LlmRequest) -> Result<LlmResponse, LlmError> {
            unimplemented!()
        }
    }

    fn agent_with_rules(mode: &str, rules: RuleSet) -> Agent {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let config = AgentConfig {
            permission_mode: Arc::new(Mutex::new(mode.to_string())),
            permission_rules: rules,
            ..AgentConfig::default()
        };
        Agent::new(
            Arc::new(MockLlmProvider),
            ToolRegistry::new(),
            config,
            tx,
            Arc::new(std::sync::RwLock::new(AgentRegistry::load(
                &std::env::temp_dir(),
            ))),
        )
    }

    #[tokio::test]
    async fn preflight_deny_rule_blocks_bypass_permissions_before_lookup() {
        let mut rules = RuleSet::empty();
        rules.always_deny.push(ToolRule {
            tool: "Bash".to_string(),
            pattern: "*".to_string(),
        });
        let mut agent = agent_with_rules("bypassPermissions", rules);
        let pending = [PendingToolCall {
            id: "tool-1".to_string(),
            name: "Bash".to_string(),
            input_json: r#"{"command":"cargo test"}"#.to_string(),
        }];

        let allowed = agent.preflight_tools(&pending, AgentMode::Normal).await;

        assert!(allowed.is_empty());
        let tool_result = &agent.state.messages[0]["content"][0];
        assert_eq!(tool_result["tool_use_id"], "tool-1");
        assert_eq!(tool_result["is_error"], true);
        assert!(
            tool_result["content"]
                .as_str()
                .unwrap_or_default()
                .contains("Blocked by deny rule")
        );
    }

    #[tokio::test]
    async fn preflight_deny_rule_blocks_dont_ask_mode() {
        let mut rules = RuleSet::empty();
        rules.always_deny.push(ToolRule {
            tool: "Bash".to_string(),
            pattern: "*".to_string(),
        });
        let mut agent = agent_with_rules("dontAsk", rules);
        let pending = [PendingToolCall {
            id: "tool-1".to_string(),
            name: "Bash".to_string(),
            input_json: r#"{"command":"cargo test"}"#.to_string(),
        }];

        let allowed = agent.preflight_tools(&pending, AgentMode::Normal).await;

        assert!(allowed.is_empty());
        let tool_result = &agent.state.messages[0]["content"][0];
        assert_eq!(tool_result["tool_use_id"], "tool-1");
        assert_eq!(tool_result["is_error"], true);
        assert!(
            tool_result["content"]
                .as_str()
                .unwrap_or_default()
                .contains("Blocked by deny rule")
        );
    }
}
