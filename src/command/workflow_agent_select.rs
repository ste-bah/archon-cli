use archon_workflow::{ProviderTier, StageKind, StageRunRequest};

pub(super) fn select_workflow_agent_key(
    request: &StageRunRequest,
    available_agents: &[String],
) -> String {
    if let Some(agent) = request
        .agent
        .as_deref()
        .filter(|agent| agent_available(available_agents, agent))
    {
        return agent.to_string();
    }

    let text = format!("{} {}", request.stage_id, request.task).to_ascii_lowercase();
    let candidates: &[&str] = if matches!(request.stage_kind, StageKind::Implementation) {
        implementation_candidates(&text)
    } else if command_like(&text) || request.provider_tier == ProviderTier::Local {
        &[
            "test-runner",
            "tester",
            "local-coder",
            "coder",
            "general-purpose",
        ]
    } else if review_like(&text) || request.provider_tier == ProviderTier::Critic {
        &[
            "sherlock-holmes",
            "code-reviewer",
            "reviewer",
            "general-purpose",
        ]
    } else if request.provider_tier == ProviderTier::Researcher {
        &["researcher", "explore", "general-purpose"]
    } else if request.provider_tier == ProviderTier::Reducer
        || matches!(request.stage_kind, StageKind::Reduce)
    {
        &["doc-writer", "researcher", "general-purpose"]
    } else if request.provider_tier == ProviderTier::Planner {
        &["planner", "plan", "system-designer", "general-purpose"]
    } else {
        &["general-purpose"]
    };
    first_available(candidates, available_agents)
}

fn implementation_candidates(text: &str) -> &'static [&'static str] {
    if text.contains("rust") || text.contains("cargo") || text.contains(".rs") {
        &[
            "rust-systems-coder",
            "local-coder",
            "coder",
            "general-purpose",
        ]
    } else if text.contains("frontend") || text.contains("react") || text.contains("ui") {
        &[
            "frontend-implementation-specialist",
            "coder",
            "general-purpose",
        ]
    } else if text.contains("backend") || text.contains("api") || text.contains("database") {
        &[
            "backend-implementation-specialist",
            "coder",
            "general-purpose",
        ]
    } else {
        &["coder", "local-coder", "general-purpose"]
    }
}

fn first_available(candidates: &[&str], available_agents: &[String]) -> String {
    candidates
        .iter()
        .find(|agent| agent_available(available_agents, agent))
        .copied()
        .or_else(|| available_agents.iter().map(String::as_str).next())
        .unwrap_or("general-purpose")
        .to_string()
}

fn agent_available(available_agents: &[String], agent: &str) -> bool {
    available_agents.is_empty() || available_agents.iter().any(|name| name == agent)
}

fn command_like(text: &str) -> bool {
    [
        "focused test",
        "cargo test",
        "cargo check",
        "cargo build",
        "clippy",
        "rustfmt",
        "verification",
    ]
    .iter()
    .any(|needle| text.contains(needle))
}

fn review_like(text: &str) -> bool {
    ["review", "audit", "adversarial", "critic", "quality"]
        .iter()
        .any(|needle| text.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn request(stage_kind: StageKind, tier: ProviderTier, task: &str) -> StageRunRequest {
        StageRunRequest {
            run_id: "wf-test".into(),
            stage_id: "stage".into(),
            stage_kind,
            agent: None,
            task: task.into(),
            attempt: 1,
            provider_tier: tier,
            depends_on: Vec::new(),
            input: json!({}),
        }
    }

    #[test]
    fn selects_rust_coder_for_rust_implementation() {
        let agents = vec![
            "general-purpose".into(),
            "coder".into(),
            "rust-systems-coder".into(),
        ];
        let req = request(
            StageKind::Implementation,
            ProviderTier::Coder,
            "Patch src/lib.rs and run cargo test",
        );
        assert_eq!(
            select_workflow_agent_key(&req, &agents),
            "rust-systems-coder"
        );
    }

    #[test]
    fn selects_sherlock_for_adversarial_review() {
        let agents = vec!["general-purpose".into(), "sherlock-holmes".into()];
        let req = request(StageKind::Agent, ProviderTier::Critic, "Adversarial review");
        assert_eq!(select_workflow_agent_key(&req, &agents), "sherlock-holmes");
    }

    #[test]
    fn explicit_known_agent_wins() {
        let agents = vec!["general-purpose".into(), "code-reviewer".into()];
        let mut req = request(StageKind::Agent, ProviderTier::Critic, "Review");
        req.agent = Some("code-reviewer".into());
        assert_eq!(select_workflow_agent_key(&req, &agents), "code-reviewer");
    }
}
