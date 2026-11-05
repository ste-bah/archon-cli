//! Agent identity is authored directly or bound to a declared role.
use crate::StageRunRequest;

pub fn select_workflow_agent_key(request: &StageRunRequest, available_agents: &[String]) -> String {
    if let Some(agent) = &request.agent {
        return agent.clone();
    }
    let role = request
        .input
        .get("v2_call")
        .and_then(|call| call.get("options"))
        .and_then(|options| options.get("role"))
        .and_then(serde_json::Value::as_str)
        .or_else(|| {
            request
                .input
                .get("v2_call")
                .and_then(|call| call.get("role"))
                .and_then(serde_json::Value::as_str)
        });
    if let Some(role) = role {
        if let Some(agent) = request
            .input
            .get("agent_bindings")
            .and_then(|bindings| bindings.get(role))
            .and_then(serde_json::Value::as_str)
        {
            return agent.to_string();
        }
        if available_agents.iter().any(|agent| agent == role) {
            return role.to_string();
        }
    }
    // Prefer the general default, then registry order; prose never selects a specialist.
    available_agents
        .iter()
        .find(|agent| agent.as_str() == "general-purpose")
        .or_else(|| available_agents.first())
        .cloned()
        .unwrap_or_else(|| "general-purpose".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ProviderTier, StageKind};
    use serde_json::json;
    fn request() -> StageRunRequest {
        StageRunRequest {
            run_id: "test".into(),
            stage_id: "arbitrary".into(),
            stage_kind: StageKind::Agent,
            agent: None,
            task: "arbitrary".into(),
            attempt: 1,
            provider_tier: ProviderTier::Coder,
            depends_on: vec![],
            input: json!({}),
        }
    }
    #[test]
    fn explicit_missing_agent_is_not_silently_substituted() {
        let mut req = request();
        req.agent = Some("authored-worker".into());
        assert_eq!(
            select_workflow_agent_key(&req, &["default".into()]),
            "authored-worker"
        );
    }
    #[test]
    fn routing_uses_authored_bindings_and_ignores_prose() {
        let mut req = request();
        req.input = json!({"v2_call": {"options": {"role": "inspector"}},
            "agent_bindings": {"inspector": "worker-b"}});
        assert_eq!(
            select_workflow_agent_key(&req, &["worker-a".into(), "worker-b".into()]),
            "worker-b"
        );
        req.input = json!({"v2_call": {"role": "worker-b"}});
        assert_eq!(
            select_workflow_agent_key(&req, &["worker-a".into(), "worker-b".into()]),
            "worker-b"
        );
        req.input = json!({});
        req.task = "review verify implementation".into();
        assert_eq!(
            select_workflow_agent_key(&req, &["worker-a".into()]),
            "worker-a"
        );
    }
}
