use futures_util::future::join_all;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::learning::integration::{LearningContext, LearningIntegration};
use crate::runner::{AgentExecutionRequest, AgentInfo, LlmClient, PipelineType, ToolAccessLevel};

use super::super::agents::{GameTheoryAgent, GameTheoryToolAccess};
use super::super::errors::GameTheoryError;
use super::super::fingerprint::GameTheoryFingerprint;
use super::super::prompt_builder;
use super::super::registry::GAMETHEORY_AGENTS;
use super::super::routing::RoutingDecision;
#[cfg(test)]
use super::MemoryRecallAudit;
use super::costs::{agent_tier, estimate_llm_cost_usd};
use super::memory_context::recall_prior_context_for_agent;
use super::types::{SpecialistCallOutput, SpecialistExecutionOutcome};
use super::{GameTheoryMemoryContext, GameTheoryRunOptions};

/// Test-only deterministic specialist fixture with failure isolation.
#[cfg(test)]
pub(super) fn execute_test_specialist_fixture(
    routing: &RoutingDecision,
    fingerprint: &GameTheoryFingerprint,
    situation: &str,
    memory_ctx: &GameTheoryMemoryContext,
) -> (
    HashMap<String, String>,
    Vec<(String, String)>,
    Vec<MemoryRecallAudit>,
) {
    let outcome = execute_test_specialist_fixture_with_options(
        routing,
        fingerprint,
        situation,
        memory_ctx,
        &GameTheoryRunOptions::default(),
    );
    (outcome.outputs, outcome.failed, outcome.memory_audits)
}

#[cfg(test)]
fn execute_test_specialist_fixture_with_options(
    routing: &RoutingDecision,
    fingerprint: &GameTheoryFingerprint,
    situation: &str,
    memory_ctx: &GameTheoryMemoryContext,
    options: &GameTheoryRunOptions,
) -> SpecialistExecutionOutcome {
    let fingerprint_summary = prompt_builder::fingerprint_summary_text(fingerprint);
    let mut outcome = SpecialistExecutionOutcome::default();

    for agent_key in &routing.enabled_specialists {
        if outcome.total_cost_usd >= options.budget_usd {
            outcome.budget_exceeded = true;
            break;
        }
        outcome.max_observed_concurrent = outcome.max_observed_concurrent.max(1);
        let recalled = recall_prior_context_for_agent(agent_key, memory_ctx);
        outcome.memory_audits.push(recalled.audit.clone());
        let result = execute_single_specialist_fixture(
            agent_key,
            situation,
            &fingerprint_summary,
            &recalled.text,
        );

        match result {
            Ok(output) => {
                outcome.outputs.insert(agent_key.clone(), output);
                outcome.costs_usd.insert(agent_key.clone(), 0.0);
            }
            Err(err_msg) => {
                outcome.failed.push((agent_key.clone(), err_msg));
            }
        }
    }

    outcome
}

/// Execute a single deterministic specialist fixture.
///
/// Test hook: if `agent_key` ends with `-FORCE-FAIL-FOR-TEST`, returns Err.
#[cfg(test)]
fn execute_single_specialist_fixture(
    agent_key: &str,
    situation: &str,
    fingerprint_summary: &str,
    prior_context: &str,
) -> std::result::Result<String, String> {
    if agent_key.ends_with("-FORCE-FAIL-FOR-TEST") {
        return Err(format!("forced failure for test: {agent_key}"));
    }

    let _prompt = prompt_builder::build_specialist_prompt_with_prior_context(
        agent_key,
        agent_key,
        situation,
        fingerprint_summary,
        prior_context,
        &[],
    );

    let prior_context_section = if prior_context.trim().is_empty() {
        String::new()
    } else {
        format!("\n\n**Prior Context:**\n\n{prior_context}")
    };

    Ok(format!(
        "## {agent_key} - Fixture Analysis\n\n\
         **Situation:** {situation}\n\n\
         **Fingerprint:** {fp_summary}{prior_context_section}\n\n\
         *Test-only deterministic fixture output.*",
        fp_summary = fingerprint_summary,
    ))
}

/// Execute real LLM-backed specialist agents.
///
/// Each enabled specialist is spawned as a separate LLM call. Failures are
/// isolated — a single specialist failure does not abort the others.
///
/// Returns `(successful_outputs, failed_specialists)`.
#[cfg(test)]
pub(super) async fn execute_specialists_real(
    llm: &dyn LlmClient,
    routing: &RoutingDecision,
    fingerprint: &GameTheoryFingerprint,
    situation: &str,
    memory_ctx: &GameTheoryMemoryContext,
) -> Result<
    (
        HashMap<String, String>,
        Vec<(String, String)>,
        Vec<MemoryRecallAudit>,
    ),
    GameTheoryError,
> {
    let outcome = execute_specialists_real_with_options(
        llm,
        routing,
        fingerprint,
        situation,
        memory_ctx,
        &GameTheoryRunOptions::default(),
        None,
    )
    .await?;
    Ok((outcome.outputs, outcome.failed, outcome.memory_audits))
}

pub(super) async fn execute_specialists_real_with_options(
    llm: &dyn LlmClient,
    routing: &RoutingDecision,
    fingerprint: &GameTheoryFingerprint,
    situation: &str,
    memory_ctx: &GameTheoryMemoryContext,
    options: &GameTheoryRunOptions,
    mut learning: Option<&mut LearningIntegration>,
) -> Result<SpecialistExecutionOutcome, GameTheoryError> {
    let fingerprint_summary = prompt_builder::fingerprint_summary_text(fingerprint);
    let mut outcome = SpecialistExecutionOutcome::default();
    let max_concurrent = options.max_concurrent.max(1);

    let system = vec![serde_json::json!({
        "type": "text",
        "text": "You are a game-theory analysis specialist. Analyze the given strategic situation from your specialist perspective and produce a detailed markdown report section. Focus on your area of expertise. Output ONLY the report content, no preamble."
    })];

    for wave in routing.enabled_specialists.chunks(max_concurrent) {
        if outcome.total_cost_usd >= options.budget_usd {
            outcome.budget_exceeded = true;
            break;
        }

        let remaining_budget = options.budget_usd - outcome.total_cost_usd;
        let affordable_slots = if outcome.total_cost_usd == 0.0 && remaining_budget > 0.0 {
            1.max(wave.len())
        } else if remaining_budget > 0.0 {
            wave.len()
        } else {
            0
        };
        let active_wave = &wave[..affordable_slots.min(wave.len())];
        if active_wave.is_empty() {
            outcome.budget_exceeded = true;
            break;
        }
        outcome.max_observed_concurrent = outcome.max_observed_concurrent.max(active_wave.len());

        let mut learning_contexts = HashMap::new();
        if let Some(li) = learning.as_deref_mut() {
            for agent_key in active_wave {
                let phase = agent_tier(agent_key)
                    .map(|tier| format!("tier{tier}"))
                    .unwrap_or_else(|| "specialist".to_string());
                let ctx = li.on_agent_start(agent_key, &phase, situation, &fingerprint.run_id);
                learning_contexts.insert(agent_key.clone(), ctx);
            }
        }

        let calls = active_wave.iter().map(|agent_key| {
            execute_specialist_call(
                llm,
                &fingerprint.run_id,
                agent_key,
                situation,
                &fingerprint_summary,
                memory_ctx,
                &system,
                learning_contexts.get(agent_key),
            )
        });
        let results = join_all(calls).await;
        for result in results {
            if let Some(li) = learning.as_deref_mut() {
                let quality = if result.output.is_some() { 0.9 } else { 0.0 };
                let summary = result
                    .output
                    .as_deref()
                    .or(result.error.as_deref())
                    .unwrap_or_default();
                li.on_agent_complete(&result.agent_key, quality, summary);
            }
            outcome.memory_audits.push(result.audit);
            if let Some(output) = result.output {
                if let Some(tier) = agent_tier(&result.agent_key) {
                    *outcome.tier_costs_usd.entry(tier).or_insert(0.0) += result.cost_usd;
                }
                outcome.total_cost_usd += result.cost_usd;
                outcome
                    .costs_usd
                    .insert(result.agent_key.clone(), result.cost_usd);
                outcome.outputs.insert(result.agent_key, output);
            } else if let Some(error) = result.error {
                outcome.failed.push((result.agent_key, error));
            }
        }
        if outcome.total_cost_usd >= options.budget_usd {
            outcome.budget_exceeded = true;
        }
    }

    Ok(outcome)
}

async fn execute_specialist_call(
    llm: &dyn LlmClient,
    run_id: &str,
    agent_key: &str,
    situation: &str,
    fingerprint_summary: &str,
    memory_ctx: &GameTheoryMemoryContext,
    system: &[serde_json::Value],
    learning_context: Option<&LearningContext>,
) -> SpecialistCallOutput {
    let recalled = recall_prior_context_for_agent(agent_key, memory_ctx);
    let agent = registry_agent(agent_key);
    let (display_name, model, source_body) = match agent {
        Some(agent) => match load_agent_prompt_body(agent) {
            Ok(body) => (agent.display_name, agent.model, body),
            Err(error) => {
                return SpecialistCallOutput {
                    agent_key: agent_key.to_string(),
                    output: None,
                    error: Some(error.to_string()),
                    audit: recalled.audit,
                    cost_usd: 0.0,
                };
            }
        },
        None => {
            tracing::warn!(
                agent_key,
                model = "claude-sonnet-4-6",
                "gametheory.agent.model_override"
            );
            (agent_key, "claude-sonnet-4-6", String::new())
        }
    };
    if agent_key.ends_with("-FORCE-FAIL-FOR-TEST") {
        return SpecialistCallOutput {
            agent_key: agent_key.to_string(),
            output: None,
            error: Some(format!("forced failure for test: {agent_key}")),
            audit: recalled.audit,
            cost_usd: 0.0,
        };
    }

    let combined_prior_context =
        combine_prior_and_learning_context(&recalled.text, learning_context);
    let prompt = prompt_builder::build_specialist_prompt_with_template(
        agent_key,
        display_name,
        &source_body,
        situation,
        fingerprint_summary,
        &combined_prior_context,
        &[],
    );
    let messages = vec![serde_json::json!({
        "role": "user",
        "content": prompt
    })];

    let tool_access_level = agent
        .map(|agent| {
            if agent
                .tool_access
                .iter()
                .any(|tool| matches!(tool, GameTheoryToolAccess::Write))
            {
                ToolAccessLevel::Full
            } else {
                ToolAccessLevel::ReadOnly
            }
        })
        .unwrap_or(ToolAccessLevel::ReadOnly);
    let allowed_tools = agent
        .map(|agent| agent.allowed_tool_names())
        .unwrap_or_else(|| {
            vec![
                "Read".into(),
                "Grep".into(),
                "Glob".into(),
                "memory_recall".into(),
            ]
        });

    match llm
        .run_agent(AgentExecutionRequest {
            session_id: run_id.to_string(),
            pipeline_type: PipelineType::GameTheory,
            task: situation.to_string(),
            cwd: None,
            ordinal: 0,
            attempt: 1,
            agent: AgentInfo {
                key: agent_key.to_string(),
                display_name: display_name.to_string(),
                model: model.to_string(),
                phase: agent.map(|agent| agent.tier as u32).unwrap_or(0),
                critical: false,
                parallelizable: true,
                quality_threshold: 0.5,
                tool_access_level,
            },
            messages,
            system: system.to_vec(),
            tools: vec![],
            allowed_tools,
        })
        .await
    {
        Ok(response) => SpecialistCallOutput {
            agent_key: agent_key.to_string(),
            output: Some(response.content),
            error: None,
            audit: recalled.audit,
            cost_usd: estimate_llm_cost_usd(model, response.tokens_in, response.tokens_out),
        },
        Err(e) => SpecialistCallOutput {
            agent_key: agent_key.to_string(),
            output: None,
            error: Some(format!("LLM error: {e}")),
            audit: recalled.audit,
            cost_usd: 0.0,
        },
    }
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

fn registry_agent(agent_key: &str) -> Option<&'static GameTheoryAgent> {
    GAMETHEORY_AGENTS
        .iter()
        .find(|agent| agent.key == agent_key)
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")))
        .to_path_buf()
}

fn load_agent_prompt_body(agent: &GameTheoryAgent) -> Result<String, GameTheoryError> {
    let path = workspace_root().join(agent.prompt_source_path);
    if !path.is_file() {
        return Err(GameTheoryError::MissingAgentFile {
            path: path.display().to_string(),
        });
    }
    let content = std::fs::read_to_string(&path).map_err(|error| GameTheoryError::Io {
        message: format!("read {} failed: {error}", path.display()),
    })?;
    let (_frontmatter, body) =
        crate::agent_loader::parse_frontmatter(&content).map_err(|error| {
            GameTheoryError::Validation {
                message: format!("parse {} failed: {error}", path.display()),
            }
        })?;
    if body.trim().is_empty() {
        return Err(GameTheoryError::Validation {
            message: format!("agent source file has empty body: {}", path.display()),
        });
    }
    Ok(body)
}

#[cfg(test)]
mod prompt_source_tests {
    use super::*;

    #[test]
    fn load_agent_prompt_body_fails_when_declared_source_file_is_missing() {
        static ACCESS: &[super::super::super::agents::GameTheoryToolAccess] = &[];
        let agent = GameTheoryAgent {
            key: "missing-source-agent",
            display_name: "Missing Source Agent",
            tier: 1,
            file: "missing-source-agent.md",
            memory_keys: &[],
            output_artifacts: &[],
            prompt_source_path: ".archon/agents/gametheory/does-not-exist-for-test.md",
            tool_access: ACCESS,
            model: "opus",
            condition: None,
            depends_on: &[],
            mandatory: false,
        };

        assert!(matches!(
            load_agent_prompt_body(&agent),
            Err(GameTheoryError::MissingAgentFile { .. })
        ));
    }
}
