//! The trusted fixed raw-outcome call: one exact host-owned tool policy, no
//! structured reply contract, the provider's content returned as it is.
use super::*;

impl LiveV2AgentClient {
    pub(in super::super) async fn run_agent_raw_request(
        &self,
        request: &WorkflowV2AgentRequest,
        prompt: String,
    ) -> std::result::Result<archon_workflow::WorkflowAgentOutcome, WorkflowV2AgentError> {
        let tools = self.fixed_raw_tool_policy.as_ref().ok_or_else(|| {
            WorkflowV2AgentError::Transport(
                "fixed raw outcome call has no host-owned tool policy".to_string(),
            )
        })?;
        if tools.is_empty() {
            return Err(WorkflowV2AgentError::Transport(
                "fixed raw outcome tool policy is empty".to_string(),
            ));
        }
        let stage_request = stage_request_for_v2_agent(
            &self.run_id,
            self.provider_tier,
            self.target_repository_root.clone(),
            request,
        );
        let model_alias = tier_model_alias(self.provider_tier).to_string();
        let agent = workflow_agent(&stage_request, &model_alias, &self.agent_names);
        let session_id = workflow_agent_session_id(&stage_request);
        call_sessions::note_session(&self.run_id, &request.call.id, &session_id);
        let ordinal = workflow_agent_ordinal(&stage_request);
        let mut allowed_tools = Vec::with_capacity(tools.len() + 1);
        allowed_tools.push(EXACT_TOOL_POLICY_MARKER.to_string());
        allowed_tools.extend(tools.iter().cloned());
        let call = WorkflowAgentCall {
            session_id,
            task: request.task.clone(),
            cwd: request_target_repository_root(&stage_request),
            ordinal,
            attempt: stage_request.attempt as usize,
            agent,
            messages: vec![serde_json::json!({ "role": "user", "content": prompt })],
            system: Vec::new(),
            tools: Vec::new(),
            allowed_tools,
            timeout_secs: self.timeout_secs,
            disable_auto_background: true,
            read_roots: Vec::new(),
            write_roots: Vec::new(),
            provider_env: self
                .provider_env_resolution
                .clone()
                .map(WorkflowProviderEnv::new),
        };
        let attempt = archon_tools::read_boundary::scope(
            archon_leann::language::default_exclude_patterns(),
            run_agent_with_transient_retry(&self.llm, call, |_attempt| {
                structured_trace::note_lost(structured_trace::RETRIED);
                async { Ok(()) }
            }),
        );
        let outcome = author_attempt_deadline(self.timeout_secs, attempt).await?;
        outcome.map_err(|error| WorkflowV2AgentError::from_call_error(&error))
    }
}
