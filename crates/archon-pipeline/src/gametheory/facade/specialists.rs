use futures_util::future::join_all;
#[cfg(test)]
use std::collections::HashMap;

use crate::runner::LlmClient;

use super::super::errors::GameTheoryError;
use super::super::fingerprint::GameTheoryFingerprint;
use super::super::prompt_builder;
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

        let calls = active_wave.iter().map(|agent_key| {
            execute_specialist_call(
                llm,
                agent_key,
                situation,
                &fingerprint_summary,
                memory_ctx,
                &system,
            )
        });
        let results = join_all(calls).await;
        for result in results {
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
    agent_key: &str,
    situation: &str,
    fingerprint_summary: &str,
    memory_ctx: &GameTheoryMemoryContext,
    system: &[serde_json::Value],
) -> SpecialistCallOutput {
    let recalled = recall_prior_context_for_agent(agent_key, memory_ctx);
    if agent_key.ends_with("-FORCE-FAIL-FOR-TEST") {
        return SpecialistCallOutput {
            agent_key: agent_key.to_string(),
            output: None,
            error: Some(format!("forced failure for test: {agent_key}")),
            audit: recalled.audit,
            cost_usd: 0.0,
        };
    }

    let prompt = prompt_builder::build_specialist_prompt_with_prior_context(
        agent_key,
        agent_key,
        situation,
        fingerprint_summary,
        &recalled.text,
        &[],
    );
    let messages = vec![serde_json::json!({
        "role": "user",
        "content": prompt
    })];

    match llm
        .send_message(messages, system.to_vec(), vec![], "claude-sonnet-4-6")
        .await
    {
        Ok(response) => SpecialistCallOutput {
            agent_key: agent_key.to_string(),
            output: Some(response.content),
            error: None,
            audit: recalled.audit,
            cost_usd: estimate_llm_cost_usd(
                "claude-sonnet-4-6",
                response.tokens_in,
                response.tokens_out,
            ),
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
