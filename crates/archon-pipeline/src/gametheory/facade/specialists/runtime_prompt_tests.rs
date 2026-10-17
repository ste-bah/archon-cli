use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

pub(super) fn test_project_root_for(
    routing: &super::super::super::routing::RoutingDecision,
) -> Result<std::path::PathBuf, GameTheoryError> {
    let project_root = std::env::temp_dir().join(format!(
        "archon-specialist-fixture-{}",
        uuid::Uuid::new_v4()
    ));
    for agent_key in &routing.enabled_specialists {
        if let Some(agent) = super::registry_agent(agent_key) {
            let path = project_root.join(agent.prompt_source_path);
            std::fs::create_dir_all(path.parent().expect("prompt path has parent")).map_err(
                |error| GameTheoryError::Io {
                    message: error.to_string(),
                },
            )?;
            std::fs::write(
                path,
                format!(
                    "---\nname: {}\n---\nNash-Seeker test prompt for {}.",
                    agent.key, agent.key
                ),
            )
            .map_err(|error| GameTheoryError::Io {
                message: error.to_string(),
            })?;
        }
    }
    Ok(project_root)
}

#[tokio::test]
async fn runtime_prompt_uses_custom_project_agent_asset() {
    let project = tempfile::tempdir().unwrap();
    let agent = super::registry_agent("nash-equilibrium-finder").unwrap();
    let prompt_path = project.path().join(agent.prompt_source_path);
    std::fs::create_dir_all(prompt_path.parent().unwrap()).unwrap();
    std::fs::write(
        prompt_path,
        "---\nname: project override\n---\nPROJECT-SPECIFIC GAME THEORY PROMPT",
    )
    .unwrap();

    crate::validate_builtin_agent_asset(
        crate::runner::PipelineType::GameTheory,
        agent.key,
        project.path(),
    )
    .unwrap();

    let body = super::prompt::load_agent_prompt_body_at(agent, project.path()).unwrap();
    let prompt = super::super::super::prompt_builder::build_specialist_prompt_with_template(
        agent.key,
        agent.display_name,
        &body,
        "A custom project situation.",
        "fingerprint",
        "",
        &[],
    )
    .unwrap();

    assert!(prompt.contains("PROJECT-SPECIFIC GAME THEORY PROMPT"));

    let llm = CountingClient::default();
    let result = super::execute_specialist_call(
        &llm,
        "run-custom-prompt",
        agent.key,
        ("A custom project situation.", "fingerprint"),
        &GameTheoryMemoryContext::default(),
        &[],
        None,
        project.path(),
    )
    .await;
    assert!(result.error.is_some());
    let messages = llm.messages.lock().unwrap();
    assert!(messages.contains("PROJECT-SPECIFIC GAME THEORY PROMPT"));
}

#[derive(Default)]
struct CountingClient {
    calls: AtomicUsize,
    system: std::sync::Mutex<String>,
    messages: std::sync::Mutex<String>,
}

#[async_trait::async_trait]
impl LlmClient for CountingClient {
    async fn send_message(
        &self,
        messages: Vec<serde_json::Value>,
        system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> anyhow::Result<crate::runner::LlmResponse> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        *self.system.lock().unwrap() = system
            .iter()
            .filter_map(|block| block.get("text").and_then(serde_json::Value::as_str))
            .collect::<Vec<_>>()
            .join("\n");
        *self.messages.lock().unwrap() = messages
            .iter()
            .map(serde_json::Value::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        anyhow::bail!("unexpected provider call")
    }
}

#[tokio::test]
async fn undefined_specialist_definition_fails_before_provider_call() {
    let client = CountingClient::default();
    let result = execute_specialist_call(
        &client,
        "run",
        "missing-runtime-specialist-for-test",
        ("situation", "fingerprint"),
        &GameTheoryMemoryContext::default(),
        &[],
        None,
        std::path::Path::new("."),
    )
    .await;
    let error = result.error.unwrap();
    assert!(error.contains("GameTheory pipeline"));
    assert!(error.contains("missing-runtime-specialist-for-test"));
    assert!(error.contains(".archon/agents/gametheory/missing-runtime-specialist-for-test.md"));
    assert!(error.contains("archon agent sync"));
    assert_eq!(client.calls.load(Ordering::SeqCst), 0);
}
