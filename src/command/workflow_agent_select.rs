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

    let text = routing_text(request);
    let candidates: &[&str] = if matches!(request.stage_kind, StageKind::Implementation) {
        implementation_candidates(&text)
    } else if request.provider_tier == ProviderTier::Local {
        command_candidates()
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
    } else if command_like(&text) {
        command_candidates()
    } else {
        &["general-purpose"]
    };
    first_available(candidates, available_agents)
}

fn implementation_candidates(text: &str) -> &'static [&'static str] {
    if has_rust_signal(text) {
        &[
            "rust-systems-coder",
            "local-coder",
            "coder",
            "general-purpose",
        ]
    } else if has_frontend_signal(text) {
        &[
            "frontend-implementation-specialist",
            "coder",
            "general-purpose",
        ]
    } else if has_backend_signal(text) {
        &[
            "backend-implementation-specialist",
            "coder",
            "general-purpose",
        ]
    } else {
        &["coder", "local-coder", "general-purpose"]
    }
}

fn command_candidates() -> &'static [&'static str] {
    &[
        "test-runner",
        "tester",
        "local-coder",
        "coder",
        "general-purpose",
    ]
}

fn routing_text(request: &StageRunRequest) -> String {
    let mut parts = vec![request.stage_id.as_str(), request.task.as_str()];
    append_value_text(&request.input, &mut parts);
    parts.join(" ").to_ascii_lowercase()
}

fn append_value_text<'a>(value: &'a serde_json::Value, parts: &mut Vec<&'a str>) {
    append_key(value, parts, "task_id");
    append_key(value, parts, "task_file");
    append_key(value, parts, "task");
    append_key(value, parts, "target_files");
    append_key(value, parts, "expected_target_files");
    append_key(value, parts, "required_tests");
    append_source_paths(value, parts);
}

fn append_key<'a>(value: &'a serde_json::Value, parts: &mut Vec<&'a str>, key: &str) {
    if let Some(next) = value.get(key) {
        append_strings(next, parts);
    }
    if let Some(item) = value
        .get("fanout_item")
        .and_then(serde_json::Value::as_object)
        && let Some(next) = item.get(key)
    {
        append_strings(next, parts);
    }
}

fn append_source_paths<'a>(value: &'a serde_json::Value, parts: &mut Vec<&'a str>) {
    let Some(sources) = value
        .get("source_files")
        .and_then(serde_json::Value::as_array)
    else {
        return;
    };
    for source in sources {
        append_key(source, parts, "path");
        append_key(source, parts, "absolute_path");
    }
}

fn append_strings<'a>(value: &'a serde_json::Value, parts: &mut Vec<&'a str>) {
    match value {
        serde_json::Value::String(value) => parts.push(value.as_str()),
        serde_json::Value::Array(values) => {
            for value in values {
                append_strings(value, parts);
            }
        }
        _ => {}
    }
}

fn has_rust_signal(text: &str) -> bool {
    text.contains(".rs") || has_term(text, "rust") || has_term(text, "cargo")
}

fn has_frontend_signal(text: &str) -> bool {
    ["frontend", "front-end", "react", "tsx", "jsx", "vue", "ui"]
        .iter()
        .any(|term| has_term(text, term))
}

fn has_backend_signal(text: &str) -> bool {
    ["backend", "api", "database", "service"]
        .iter()
        .any(|term| has_term(text, term))
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

fn has_term(text: &str, term: &str) -> bool {
    text.split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '-'))
        .any(|token| token == term)
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

    fn with_input(mut request: StageRunRequest, input: serde_json::Value) -> StageRunRequest {
        request.input = input;
        request
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
    fn selects_rust_coder_from_fanout_target_files() {
        let agents = vec![
            "general-purpose".into(),
            "coder".into(),
            "frontend-implementation-specialist".into(),
            "rust-systems-coder".into(),
        ];
        let req = with_input(
            request(
                StageKind::Implementation,
                ProviderTier::Coder,
                "Implement T001 only and confirm it is acceptable before continuing.",
            ),
            json!({
                "fanout_item": {
                    "task_id": "TASK-GENERIC-001",
                    "target_files": ["crates/example/src/lib.rs"]
                }
            }),
        );
        assert_eq!(
            select_workflow_agent_key(&req, &agents),
            "rust-systems-coder"
        );
    }

    #[test]
    fn continuing_does_not_trigger_ui_frontend_agent() {
        let agents = vec![
            "general-purpose".into(),
            "coder".into(),
            "frontend-implementation-specialist".into(),
        ];
        let req = request(
            StageKind::Implementation,
            ProviderTier::Coder,
            "Confirm T001 is acceptable before continuing.",
        );
        assert_eq!(select_workflow_agent_key(&req, &agents), "coder");
    }

    #[test]
    fn real_ui_token_still_selects_frontend_agent() {
        let agents = vec![
            "general-purpose".into(),
            "coder".into(),
            "frontend-implementation-specialist".into(),
        ];
        let req = request(
            StageKind::Implementation,
            ProviderTier::Coder,
            "Implement the React UI route.",
        );
        assert_eq!(
            select_workflow_agent_key(&req, &agents),
            "frontend-implementation-specialist"
        );
    }

    #[test]
    fn repository_state_does_not_trigger_backend_agent() {
        let agents = vec![
            "general-purpose".into(),
            "coder".into(),
            "backend-implementation-specialist".into(),
        ];
        let req = request(
            StageKind::Implementation,
            ProviderTier::Coder,
            "Inspect current repository state and implement missing work.",
        );
        assert_eq!(select_workflow_agent_key(&req, &agents), "coder");
    }

    #[test]
    fn planner_discovery_with_verification_text_stays_planner() {
        let agents = vec![
            "general-purpose".into(),
            "planner".into(),
            "test-runner".into(),
        ];
        let req = request(
            StageKind::Agent,
            ProviderTier::Planner,
            "Inspect the repository and summarize focused verification strategy.",
        );
        assert_eq!(select_workflow_agent_key(&req, &agents), "planner");
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
