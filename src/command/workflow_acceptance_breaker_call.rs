//! Call budgets and a freeze-wide circuit for transient failures.
use super::*;
use crate::command::workflow_live::EXACT_TOOL_POLICY_MARKER;
use crate::command::workflow_task_set::judge;
use archon_core::agents::harness::{HOST_READ_ONLY_TOOLS, REQUIREMENT_BREAKER_AGENT};
use archon_workflow::llm_client_port::{
    WorkflowAgentCall, WorkflowAgentSpec, WorkflowAgentToolAccess,
};
use std::time::{Duration, Instant};
pub(super) struct Circuit {
    pub(super) deadline: Instant,
    pub(super) calls: usize,
    pub(super) failed: bool,
    pub(super) pair: usize,
    pair_calls: BTreeMap<usize, usize>,
    limits: archon_core::config::BreakerCallLimits,
}
impl Circuit {
    pub(super) fn new(seconds: u64, limits: &archon_core::config::BreakerCallLimits) -> Self {
        Self {
            deadline: Instant::now() + Duration::from_secs(seconds.min(7200)),
            calls: 0,
            failed: false,
            pair: 0,
            pair_calls: BTreeMap::new(),
            limits: limits.clone(),
        }
    }
    pub(super) fn remaining(&mut self) -> Result<Duration, (UnavailableCause, String)> {
        if self
            .limits
            .global
            .finite()
            .is_some_and(|n| self.calls as u64 >= n)
            || self
                .limits
                .per_pair
                .finite()
                .is_some_and(|n| self.pair_calls.get(&self.pair).copied().unwrap_or(0) as u64 >= n)
        {
            return Err((UnavailableCause::Budget, "call_cap_exhausted".into()));
        }
        if self.failed || Instant::now() >= self.deadline {
            return Err((
                UnavailableCause::Transient,
                "call_budget_unavailable".into(),
            ));
        }
        self.calls += 1;
        *self.pair_calls.entry(self.pair).or_default() += 1;
        Ok(self.deadline.saturating_duration_since(Instant::now()))
    }
    pub(super) async fn author(
        &mut self,
        client: &dyn WorkflowLlmClient,
        view: &BaseView,
        prompt: &str,
        model: &str,
        attempt: u32,
    ) -> Result<String, (UnavailableCause, String)> {
        let duration = self.remaining()?;
        if let Some(Err(reason)) = client.probe_agent_read(view.root()) {
            self.failed = true;
            return Err((UnavailableCause::Transient, reason));
        }
        let call = agent_call(view, prompt, model, attempt, duration.as_secs());
        match tokio::time::timeout(duration, client.run_agent(call)).await {
            Ok(Ok(reply)) if judge::reply_is_complete(&reply) => Ok(reply.content),
            Ok(Ok(_)) => Err((UnavailableCause::Unusable, "incomplete_reply".into())),
            Ok(Err(e)) => {
                self.failed = true;
                Err((UnavailableCause::Transient, e.to_string()))
            }
            Err(_) => {
                self.failed = true;
                Err((UnavailableCause::Transient, "deadline_elapsed".into()))
            }
        }
    }
    pub(super) async fn judge(
        &mut self,
        client: &dyn WorkflowLlmClient,
        prompt: &str,
        model: &str,
    ) -> Result<serde_json::Value, (UnavailableCause, String)> {
        {
            let mut messages = vec![serde_json::json!({"role":"user","content":prompt})];
            let mut partial = String::new();
            for _ in 0..4 {
                let duration = self.remaining()?;
                let outcome = match tokio::time::timeout(
                    duration,
                    client.send_message_with_temperature(
                        messages.clone(),
                        vec![],
                        vec![],
                        model,
                        0.0,
                    ),
                )
                .await
                {
                    Ok(Ok(r)) => r,
                    Ok(Err(e)) => {
                        self.failed = true;
                        return Err((UnavailableCause::Transient, e.to_string()));
                    }
                    Err(_) => {
                        self.failed = true;
                        return Err((UnavailableCause::Transient, "deadline_elapsed".into()));
                    }
                };
                match judge::step(&partial, !partial.is_empty(), &outcome) {
                    judge::Step::Done(text) => {
                        if let Ok(value) = prompt::object(&text) {
                            return Ok(value);
                        }
                        break;
                    }
                    judge::Step::Continue(text) => {
                        partial = text;
                        messages = vec![
                            serde_json::json!({"role":"user","content":prompt}),
                            serde_json::json!({"role":"assistant","content":partial}),
                            serde_json::json!({"role":"user","content":"Continue the same JSON object from where it stopped. Do not restart it."}),
                        ];
                    }
                    _ => break,
                }
            }
        }
        Err((UnavailableCause::Unusable, "unusable_reply".into()))
    }
}

pub(super) fn agent_call(
    view: &BaseView,
    prompt: &str,
    model: &str,
    attempt: u32,
    seconds: u64,
) -> WorkflowAgentCall {
    WorkflowAgentCall {
        session_id: format!("{REQUIREMENT_BREAKER_AGENT}-{}", uuid::Uuid::new_v4()),
        task: prompt.into(),
        cwd: Some(view.root().into()),
        ordinal: 0,
        attempt: attempt as usize,
        agent: WorkflowAgentSpec {
            key: REQUIREMENT_BREAKER_AGENT.into(),
            display_name: "requirement breaker".into(),
            model: model.into(),
            phase: 0,
            critical: false,
            parallelizable: false,
            quality_threshold: 0.5,
            tool_access: WorkflowAgentToolAccess::ReadOnly,
        },
        messages: vec![serde_json::json!({"role":"user","content":prompt})],
        system: vec![],
        tools: vec![],
        allowed_tools: std::iter::once(EXACT_TOOL_POLICY_MARKER)
            .chain(HOST_READ_ONLY_TOOLS)
            .map(str::to_string)
            .collect(),
        timeout_secs: Some(seconds),
        disable_auto_background: true,
        read_roots: vec![],
        write_roots: vec![],
        provider_env: None,
    }
}
