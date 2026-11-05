//! Which authored agent key runs a workflow stage.

use crate::{ProviderTier, StageKind, StageRunRequest};

pub fn select_workflow_agent_key(request: &StageRunRequest, available_agents: &[String]) -> String {
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
        // A reduction classifies structured evidence and returns JSON. Both
        // former leaders prescribe a conflicting output of their own:
        // `doc-writer` exists to "generate technical documentation" and must
        // "not guess", so asked to repair wave-completion evidence it looked
        // for a document to write and blocked the run for having no target
        // file; `researcher` is told to emit a `research_findings:` YAML block.
        // A charter that names a different deliverable competes with the task
        // prompt for the whole stage, and the schema is what loses — routes
        // nested under `items`, protected fields dropped in a "shape" repair.
        &["reducer", "doc-writer", "researcher", "general-purpose"]
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

    /// The per-task `adversarial-review-<task>` stage is a PARALLEL read-only
    /// stage carrying `tier: critic`, which resolves to `ProviderTier::Critic`.
    /// It must route to an adversarial reviewer, not to a coder — parallel
    /// stages default to `ProviderTier::Coder` when no tier is declared, so the
    /// tier is the whole mechanism.
    #[test]
    fn per_task_adversarial_review_selects_an_adversarial_reviewer() {
        let agents = vec![
            "general-purpose".into(),
            "coder".into(),
            "sherlock-holmes".into(),
        ];
        let mut req = request(
            StageKind::Fanout,
            ProviderTier::Critic,
            "You did NOT do this work — be suspicious of it. Run read-only adversarial review of ONE task only.",
        );
        req.stage_id = "adversarial-review-TASK-TDL-001".into();
        assert_eq!(select_workflow_agent_key(&req, &agents), "sherlock-holmes");
    }

    /// `code-reviewer` ships as a PLUGIN agent (`plugins/feature-dev/agents/`,
    /// `plugins/pr-review-toolkit/agents/`) and reaches this list through
    /// `AgentRegistry::load`, which merges plugin agents into the registry.
    /// With no `sherlock-holmes` installed the plugin reviewer must be the one
    /// that runs; falling through to `general-purpose` would silently downgrade
    /// every per-task review to a non-adversarial agent.
    #[test]
    fn a_plugin_provided_reviewer_is_reachable_for_per_task_review() {
        let agents = vec!["general-purpose".into(), "code-reviewer".into()];
        let mut req = request(
            StageKind::Fanout,
            ProviderTier::Critic,
            "Run read-only adversarial review of ONE task only.",
        );
        req.stage_id = "adversarial-review-TASK-TDL-001".into();
        assert_eq!(select_workflow_agent_key(&req, &agents), "code-reviewer");
    }

    #[test]
    fn explicit_known_agent_wins() {
        let agents = vec!["general-purpose".into(), "code-reviewer".into()];
        let mut req = request(StageKind::Agent, ProviderTier::Critic, "Review");
        req.agent = Some("code-reviewer".into());
        assert_eq!(select_workflow_agent_key(&req, &agents), "code-reviewer");
    }

    /// A reduction returns JSON. `doc-writer` led this list and its charter
    /// names a different deliverable — asked to repair wave-completion
    /// evidence it answered "no actionable documentation task, target file, or
    /// requested scope was provided" and blocked the run.
    #[test]
    fn a_reduce_stage_prefers_the_reducer_agent() {
        let agents = vec![
            "doc-writer".into(),
            "researcher".into(),
            "reducer".into(),
            "general-purpose".into(),
        ];
        let req = request(
            StageKind::Reduce,
            ProviderTier::Reducer,
            "Classify failed focused verification outcomes",
        );
        assert_eq!(select_workflow_agent_key(&req, &agents), "reducer");
    }

    /// Projects without the agent keep the previous routing exactly.
    #[test]
    fn a_reduce_stage_falls_back_when_no_reducer_exists() {
        let agents = vec!["doc-writer".into(), "general-purpose".into()];
        let req = request(StageKind::Reduce, ProviderTier::Reducer, "Classify");
        assert_eq!(select_workflow_agent_key(&req, &agents), "doc-writer");
    }

    /// The tier alone routes it, whatever the stage kind says.
    #[test]
    fn the_reducer_tier_selects_the_reducer_agent() {
        let agents = vec!["reducer".into(), "doc-writer".into()];
        let req = request(StageKind::Agent, ProviderTier::Reducer, "Reduce evidence");
        assert_eq!(select_workflow_agent_key(&req, &agents), "reducer");
    }
}
