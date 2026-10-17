use anyhow::Result;
use futures_util::future::join_all;
use std::collections::HashMap;

use super::super::errors::GameTheoryError;
use super::super::fingerprint::{AxisVerdict, GameTheoryFingerprint};
use super::super::registry::GAMETHEORY_AGENTS;
use super::memory_context::recall_prior_context_for_agent;
use super::types::Tier1AgentOutput;
use super::{GameTheoryMemoryContext, MemoryRecallAudit, TIER1_MEMORY_AGENT_KEYS};
use crate::learning::integration::{LearningContext, LearningIntegration};
use crate::runner::{
    AgentExecutionRequest, AgentInfo, LlmClient, LlmResponse, PipelineType, ToolAccessLevel,
};

/// Execute real Tier 1 classification via the four mandatory Tier 1 agents.
///
/// The PRD requires the mandatory foundation agents to run as one parallel
/// wave. `game-classifier` is responsible for the machine-readable 9-axis
/// fingerprint; the other foundation outputs are executed and available for
/// audit/prompt evolution but do not overwrite the classifier JSON.
pub(super) async fn execute_tier1_real(
    llm: &dyn LlmClient,
    run_id: &str,
    situation: &str,
    now: &str,
    memory_ctx: &GameTheoryMemoryContext,
    project_root: &std::path::Path,
    mut learning: Option<&mut LearningIntegration>,
) -> Result<(GameTheoryFingerprint, Vec<MemoryRecallAudit>), GameTheoryError> {
    let mut audits = Vec::new();
    let mut prior_context_parts = Vec::new();
    for agent_key in TIER1_MEMORY_AGENT_KEYS {
        let recalled = recall_prior_context_for_agent(agent_key, memory_ctx);
        if !recalled.text.is_empty() {
            prior_context_parts.push(format!("### {agent_key}\n\n{}", recalled.text));
        }
        audits.push(recalled.audit);
    }
    let prior_context = prior_context_parts.join("\n\n---\n\n");
    let mut learning_contexts = HashMap::new();
    if let Some(li) = learning.as_deref_mut() {
        for agent_key in TIER1_MEMORY_AGENT_KEYS {
            let ctx = li.on_agent_start(agent_key, "tier1", situation, run_id);
            learning_contexts.insert((*agent_key).to_string(), ctx);
        }
    }

    let tier1_calls = TIER1_MEMORY_AGENT_KEYS.iter().map(|agent_key| {
        execute_tier1_agent(
            llm,
            run_id,
            agent_key,
            situation,
            &prior_context,
            learning_contexts.get(*agent_key),
            project_root,
        )
    });
    let responses = join_all(tier1_calls).await;
    let mut outputs = Vec::new();
    let mut failures = Vec::new();
    for response in responses {
        match response {
            Ok(output) => {
                if let Some(li) = learning.as_deref_mut() {
                    li.on_agent_complete(&output.agent_key, 0.9, &output.content);
                }
                outputs.push(output);
            }
            Err(err) => failures.push(err.to_string()),
        }
    }
    if !failures.is_empty() {
        return Err(GameTheoryError::Tier1Execution {
            message: failures.join("; "),
        });
    }

    let classifier_output = outputs
        .iter()
        .find(|output| output.agent_key == "game-classifier")
        .or_else(|| outputs.first())
        .ok_or_else(|| GameTheoryError::Tier1Execution {
            message: "no Tier 1 agent outputs were produced".to_string(),
        })?;

    let fingerprint = parse_tier1_fingerprint(run_id, now, &classifier_output.content)?;
    Ok((fingerprint, audits))
}

async fn execute_tier1_agent(
    llm: &dyn LlmClient,
    run_id: &str,
    agent_key: &str,
    situation: &str,
    prior_context: &str,
    learning_context: Option<&LearningContext>,
    project_root: &std::path::Path,
) -> Result<Tier1AgentOutput, GameTheoryError> {
    let agent = GAMETHEORY_AGENTS
        .iter()
        .find(|agent| agent.key == agent_key)
        .ok_or_else(|| GameTheoryError::Tier1Execution {
            message: format!(
                "GameTheory pipeline agent '{agent_key}' has no registry definition; run `archon agent sync`"
            ),
        })?;
    let prompt = super::specialists::load_agent_prompt_body(agent, project_root).map_err(|error| {
        GameTheoryError::Tier1Execution {
            message: format!(
                "GameTheory pipeline agent '{}' prompt '{}' failed: {error}; run `archon agent sync`",
                agent.key, agent.prompt_source_path
            ),
        }
    })?;
    execute_tier1_agent_with_prompt(
        llm,
        run_id,
        agent,
        situation,
        prior_context,
        learning_context,
        &prompt,
    )
    .await
}

async fn execute_tier1_agent_with_prompt(
    llm: &dyn LlmClient,
    run_id: &str,
    agent: &crate::gametheory::GameTheoryAgent,
    situation: &str,
    prior_context: &str,
    learning_context: Option<&LearningContext>,
    prompt: &str,
) -> Result<Tier1AgentOutput, GameTheoryError> {
    let agent_key = agent.key;
    let system = vec![serde_json::json!({
        "type": "text",
        "text": prompt
    })];

    let combined_context = combine_prior_and_learning_context(prior_context, learning_context);

    let user_content = if combined_context.is_empty() {
        format!("Classify this strategic situation as Tier 1 agent `{agent_key}`:\n\n{situation}")
    } else {
        format!(
            "Classify this strategic situation as Tier 1 agent `{agent_key}`:\n\n{situation}\n\n## Recalled Prior Context\n\n{combined_context}"
        )
    };

    let messages = vec![serde_json::json!({
        "role": "user",
        "content": user_content
    })];

    let (display_name, model, allowed_tools) = (
        agent.display_name.to_string(),
        agent.model.to_string(),
        agent.allowed_tool_names(),
    );

    let response: LlmResponse = llm
        .run_agent(AgentExecutionRequest {
            session_id: run_id.to_string(),
            pipeline_type: PipelineType::GameTheory,
            task: situation.to_string(),
            cwd: None,
            ordinal: 0,
            attempt: 1,
            agent: AgentInfo {
                key: agent_key.to_string(),
                display_name,
                model,
                phase: 1,
                critical: true,
                parallelizable: true,
                quality_threshold: 0.5,
                tool_access_level: ToolAccessLevel::ReadOnly,
            },
            messages,
            system,
            tools: vec![],
            allowed_tools,
            timeout_secs: None,
            disable_auto_background: false,
            read_roots: Vec::new(),
            write_roots: Vec::new(),
            provider_env_resolution: None,
        })
        .await
        .map_err(|e| GameTheoryError::Storage {
            message: e.to_string(),
        })?;
    Ok(Tier1AgentOutput {
        agent_key: agent_key.to_string(),
        content: response.content,
    })
}

fn combine_prior_and_learning_context(
    prior_context: &str,
    learning_context: Option<&LearningContext>,
) -> String {
    let mut parts = Vec::new();
    if !prior_context.trim().is_empty() {
        parts.push(prior_context.to_string());
    }
    if let Some(ctx) = learning_context {
        if !ctx.sona_context.is_empty() {
            parts.push(format!("## SONA Trajectory\n{}", ctx.sona_context));
        }
        if !ctx.reasoning_context.is_empty() {
            parts.push(format!("## Reasoning Context\n{}", ctx.reasoning_context));
        }
        if !ctx.desc_episodes.is_empty() {
            parts.push(format!(
                "## DESC Episodes\n{}",
                ctx.desc_episodes.join("\n\n")
            ));
        }
    }
    parts.join("\n\n---\n\n")
}

fn parse_tier1_fingerprint(
    run_id: &str,
    now: &str,
    content: &str,
) -> Result<GameTheoryFingerprint, GameTheoryError> {
    // The classifier is told to emit bare JSON and routinely fences it anyway.
    // This used to look only for a ```json fence and otherwise slice from the
    // first `{` to the end of the response, so a bare ``` fence left the
    // closing backticks attached and failed to parse every time.
    let json_str = archon_context::fenced::json_payload(content);
    if !json_str.contains('{') {
        return Err(GameTheoryError::FingerprintParse {
            message: "LLM response did not contain JSON".into(),
        });
    }

    let parsed: serde_json::Value =
        serde_json::from_str(json_str).map_err(|e| GameTheoryError::FingerprintParse {
            message: e.to_string(),
        })?;

    let get_axis = |key: &str| -> AxisVerdict {
        parsed
            .get(key)
            .map(|v| {
                AxisVerdict::new(
                    v.get("value").and_then(|x| x.as_str()).unwrap_or("unknown"),
                    v.get("confidence")
                        .and_then(|x| x.as_str())
                        .unwrap_or("low"),
                    v.get("rationale").and_then(|x| x.as_str()).unwrap_or(""),
                )
            })
            .unwrap_or_else(|| AxisVerdict::new("unknown", "low", ""))
    };

    let primary_family = parsed
        .get("primary_family")
        .and_then(|v| v.as_str())
        .unwrap_or("Strategic interaction")
        .to_string();

    let nearest_classic = parsed
        .get("nearest_classic")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    Ok(GameTheoryFingerprint {
        run_id: run_id.to_string(),
        cooperation: get_axis("cooperation"),
        payoff_sum: get_axis("payoff_sum"),
        symmetry: get_axis("symmetry"),
        timing: get_axis("timing"),
        perfect_info: get_axis("perfect_info"),
        complete_info: get_axis("complete_info"),
        cardinality: get_axis("cardinality"),
        strategy_space: get_axis("strategy_space"),
        horizon: get_axis("horizon"),
        primary_family,
        nearest_classic,
        shadow_games: vec![],
        hidden_game_scan: None,
        ambiguities: vec![],
        created_at: now.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Runtime::new().unwrap().block_on(future)
    }

    struct PromptCapture {
        calls: AtomicUsize,
        systems: Mutex<Vec<Vec<serde_json::Value>>>,
    }

    impl PromptCapture {
        fn new() -> Self {
            Self {
                calls: AtomicUsize::new(0),
                systems: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl LlmClient for PromptCapture {
        async fn send_message(
            &self,
            _messages: Vec<serde_json::Value>,
            system: Vec<serde_json::Value>,
            _tools: Vec<serde_json::Value>,
            _model: &str,
        ) -> Result<LlmResponse, anyhow::Error> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.systems.lock().unwrap().push(system);
            Ok(LlmResponse {
                content: "captured output".into(),
                tool_uses: Vec::new(),
                tokens_in: None,
                tokens_out: None,
                stop_reason: None,
            })
        }
    }

    #[test]
    fn edited_game_classifier_asset_reaches_tier1_request() {
        let root = tempfile::tempdir().unwrap();
        let agent = GAMETHEORY_AGENTS
            .iter()
            .find(|agent| agent.key == "game-classifier")
            .unwrap();
        let prompt_path = root.path().join(agent.prompt_source_path);
        std::fs::create_dir_all(prompt_path.parent().unwrap()).unwrap();
        std::fs::write(
            prompt_path,
            "---\nname: game-classifier\n---\nEdited project classifier prompt sentinel.\n",
        )
        .unwrap();

        let llm = PromptCapture::new();
        let prompt = crate::gametheory::load_agent_prompt_body(agent, root.path()).unwrap();
        block_on(execute_tier1_agent_with_prompt(
            &llm,
            "run-edited-prompt",
            agent,
            "Two firms choose prices.",
            "",
            None,
            &prompt,
        ))
        .unwrap();

        let systems = llm.systems.lock().unwrap();
        assert_eq!(llm.calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            systems[0][0]["text"],
            "Edited project classifier prompt sentinel."
        );
    }

    #[test]
    fn unknown_tier1_key_errors_before_provider_call() {
        let llm = PromptCapture::new();

        let error = block_on(execute_tier1_agent(
            &llm,
            "run-unknown-tier1",
            "unknown-tier1-agent",
            "A situation.",
            "",
            None,
            std::path::Path::new("."),
        ))
        .unwrap_err()
        .to_string();

        assert!(error.contains("GameTheory pipeline agent 'unknown-tier1-agent'"));
        assert!(error.contains("archon agent sync"));
        assert_eq!(llm.calls.load(Ordering::SeqCst), 0);
    }
}
