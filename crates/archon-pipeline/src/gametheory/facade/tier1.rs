use anyhow::Result;
use futures_util::future::join_all;

use crate::runner::{
    AgentExecutionRequest, AgentInfo, LlmClient, LlmResponse, PipelineType, ToolAccessLevel,
};

use super::super::errors::GameTheoryError;
use super::super::fingerprint::{AxisVerdict, GameTheoryFingerprint};
use super::super::registry::GAMETHEORY_AGENTS;
use super::memory_context::recall_prior_context_for_agent;
use super::types::Tier1AgentOutput;
use super::{GameTheoryMemoryContext, MemoryRecallAudit, TIER1_MEMORY_AGENT_KEYS};

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

    let tier1_calls = TIER1_MEMORY_AGENT_KEYS
        .iter()
        .map(|agent_key| execute_tier1_agent(llm, run_id, agent_key, situation, &prior_context));
    let responses = join_all(tier1_calls).await;
    let mut outputs = Vec::new();
    let mut failures = Vec::new();
    for response in responses {
        match response {
            Ok(output) => outputs.push(output),
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
) -> Result<Tier1AgentOutput, GameTheoryError> {
    let system = vec![serde_json::json!({
        "type": "text",
        "text": tier1_system_prompt(agent_key)
    })];

    let user_content = if prior_context.is_empty() {
        format!("Classify this strategic situation as Tier 1 agent `{agent_key}`:\n\n{situation}")
    } else {
        format!(
            "Classify this strategic situation as Tier 1 agent `{agent_key}`:\n\n{situation}\n\n## Recalled Prior Context\n\n{prior_context}"
        )
    };

    let messages = vec![serde_json::json!({
        "role": "user",
        "content": user_content
    })];

    let (display_name, model, allowed_tools) = GAMETHEORY_AGENTS
        .iter()
        .find(|agent| agent.key == agent_key)
        .map(|agent| {
            (
                agent.display_name.to_string(),
                agent.model.to_string(),
                agent.allowed_tool_names(),
            )
        })
        .unwrap_or_else(|| {
            (
                agent_key.to_string(),
                "claude-sonnet-4-6".to_string(),
                vec![
                    "Read".into(),
                    "Grep".into(),
                    "Glob".into(),
                    "memory_recall".into(),
                ],
            )
        });

    let response: LlmResponse = llm
        .run_agent(AgentExecutionRequest {
            session_id: run_id.to_string(),
            pipeline_type: PipelineType::GameTheory,
            task: situation.to_string(),
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

fn tier1_system_prompt(agent_key: &str) -> &'static str {
    match agent_key {
        "game-classifier" => {
            "You are the game-classifier Tier 1 foundation agent. Analyze the given strategic situation and output a JSON object with exactly these fields: cooperation (cooperative/non-cooperative), payoff_sum (zero-sum/positive-sum/variable-sum), symmetry (symmetric/asymmetric/unknown), timing (simultaneous/sequential/repeated), perfect_info (perfect/imperfect), complete_info (complete/incomplete), cardinality (2-player/n-player), strategy_space (continuous/discrete), horizon (one-shot/repeated), primary_family (short label like \"Bertrand competition\"), nearest_classic (classic game name or null). For each axis also include a confidence (low/medium/high) and a brief rationale. Output ONLY the JSON object, no markdown wrapping."
        }
        "payoff-elicitor" => {
            "You are the payoff-elicitor Tier 1 foundation agent. Identify players, incentives, payoff dimensions, likely payoff conflicts, and missing payoff assumptions. Output concise markdown."
        }
        "strategy-space-enumerator" => {
            "You are the strategy-space-enumerator Tier 1 foundation agent. Enumerate each player's feasible actions, strategies, constraints, and whether the strategy space is discrete or continuous. Output concise markdown."
        }
        "information-structure-mapper" => {
            "You are the information-structure-mapper Tier 1 foundation agent. Map who knows what, what is hidden, signalling channels, beliefs, and information asymmetries. Output concise markdown."
        }
        _ => {
            "You are a Tier 1 game-theory foundation agent. Analyze the strategic situation from your assigned perspective."
        }
    }
}

fn parse_tier1_fingerprint(
    run_id: &str,
    now: &str,
    content: &str,
) -> Result<GameTheoryFingerprint, GameTheoryError> {
    let trimmed = content.trim();
    let json_str = if let Some(start) = trimmed.find("```json") {
        let inner = &trimmed[start + 7..];
        if let Some(end) = inner.find("```") {
            &inner[..end]
        } else {
            inner
        }
    } else if let Some(start) = trimmed.find('{') {
        &trimmed[start..]
    } else {
        return Err(GameTheoryError::FingerprintParse {
            message: "LLM response did not contain JSON".into(),
        });
    };

    let parsed: serde_json::Value =
        serde_json::from_str(json_str.trim()).map_err(|e| GameTheoryError::FingerprintParse {
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
