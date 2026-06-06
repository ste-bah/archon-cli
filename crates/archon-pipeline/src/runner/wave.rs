use anyhow::Result;
use futures_util::future::join_all;

use super::quality_gate::{
    force_acceptance_reason, has_non_bypassable_quality_failure, quality_gate_acceptance,
};
use super::support::{
    append_learning_context, log_wave_agent_completed, quality_failure_reason,
    record_reflexion_failure, reindex_modified_files,
};
use super::*;
use crate::audit::types::AgentAttemptRecord;

struct PreparedWaveAgent {
    ordinal: usize,
    agent: AgentInfo,
    session: PipelineSession,
    messages: Vec<serde_json::Value>,
    system: Vec<serde_json::Value>,
    tools: Vec<serde_json::Value>,
    prompt: Option<PromptHashes>,
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn run_parallel_wave(
    facade: &dyn PipelineFacade,
    llm: &dyn LlmClient,
    session: &mut PipelineSession,
    leann: Option<&LeannIntegration>,
    reflexion: &mut Option<&mut ReflexionInjector>,
    learning: &mut Option<&mut LearningIntegration>,
    audit: &mut Option<PipelineAuditRun>,
    agents: Vec<AgentInfo>,
    options: PipelineRunOptions,
) -> Result<()> {
    let agents: Vec<AgentInfo> = agents
        .into_iter()
        .take(4)
        .filter(|agent| agent.parallelizable)
        .collect();
    if agents.is_empty() {
        return Ok(());
    }
    tracing::info!(
        session_id = %session.id,
        wave_size = agents.len(),
        agent_keys = %agents.iter().map(|agent| agent.key.as_str()).collect::<Vec<_>>().join(","),
        "pipeline.agent.parallel_wave_started"
    );

    let prepared = prepare_wave_agents(facade, session, leann, learning, audit, agents).await?;
    let completed = join_all(
        prepared
            .into_iter()
            .map(|prepared| execute_prepared_wave_agent(facade, llm, prepared)),
    )
    .await;

    for item in completed {
        let (prepared, mut result, mut quality, mut attempt, mut accepted_prompt) = match item {
            Ok((prepared, result, quality)) => {
                let accepted_prompt = prepared.prompt.clone();
                (prepared, result, quality, 1usize, accepted_prompt)
            }
            Err((prepared, error)) => {
                recover_wave_initial_failure(facade, llm, audit, prepared, error).await?
            }
        };

        let mut attempts = Vec::new();
        loop {
            let meets_threshold = quality.overall >= prepared.agent.quality_threshold;
            let failure_reason = (!meets_threshold)
                .then(|| quality_failure_reason(&prepared.agent, quality.overall));
            let effective_options = if has_non_bypassable_quality_failure(&quality) {
                PipelineRunOptions::default()
            } else {
                options
            };
            let gate = quality_gate_acceptance(
                meets_threshold,
                &prepared.agent,
                attempt,
                effective_options,
            );
            let failure_reason = if gate.force_accepted {
                Some(force_acceptance_reason(failure_reason))
            } else {
                failure_reason
            };
            if let Some(audit) = audit.as_ref() {
                attempts.push(audit.record_attempt_completed(
                    prepared.ordinal,
                    &prepared.agent,
                    attempt,
                    &result,
                    gate.accepted,
                    failure_reason.clone(),
                )?);
                if gate.force_accepted {
                    audit.record_quality_gate_force_accepted(
                        prepared.ordinal,
                        &prepared.agent,
                        attempt,
                        quality.overall,
                        failure_reason.as_deref().unwrap_or("force-accepted"),
                    )?;
                }
            }
            log_wave_agent_completed(&prepared.agent, attempt, &quality, &result);

            if gate.accepted {
                break;
            }
            if attempt >= PIPELINE_MAX_ATTEMPTS && prepared.agent.critical {
                let reason = quality_gate_failure(&prepared.agent, quality.overall, attempt);
                fail_audit(audit, &reason)?;
                return Err(anyhow::anyhow!(reason));
            }

            record_reflexion_failure(
                reflexion,
                &prepared.agent,
                attempt,
                &result,
                &quality,
                &failure_reason,
            );
            if let Some(audit) = audit.as_ref() {
                audit.record_agent_retry(
                    prepared.ordinal,
                    &prepared.agent,
                    attempt,
                    failure_reason
                        .as_deref()
                        .unwrap_or("quality threshold not met"),
                )?;
            }

            attempt += 1;
            let reflexion_section = reflexion
                .as_deref()
                .and_then(|ri| ri.inject_reflexion(&prepared.agent.key))
                .map(|ctx| ctx.formatted_prompt_section);
            match run_wave_attempt(facade, llm, audit, &prepared, attempt, reflexion_section).await
            {
                Ok((retry_result, retry_quality, retry_prompt)) => {
                    result = retry_result;
                    quality = retry_quality;
                    accepted_prompt = retry_prompt;
                }
                Err(error)
                    if is_context_window_error(&error) && attempt < PIPELINE_MAX_ATTEMPTS =>
                {
                    tracing::warn!(
                        agent_key = %prepared.agent.key,
                        attempt = attempt,
                        "pipeline parallel-wave prompt exceeded context; rebuilding with tighter retry budget"
                    );
                    continue;
                }
                Err(error) => {
                    if let Some(audit) = audit.as_ref() {
                        audit.record_attempt_failed(
                            prepared.ordinal,
                            &prepared.agent,
                            attempt,
                            &error.to_string(),
                        )?;
                    }
                    fail_audit(audit, &error.to_string())?;
                    return Err(error);
                }
            }
        }

        commit_wave_agent_completion(
            facade,
            session,
            leann,
            learning,
            audit,
            prepared,
            result,
            quality,
            attempts,
            accepted_prompt,
        )
        .await?;
    }
    Ok(())
}

async fn prepare_wave_agents(
    facade: &dyn PipelineFacade,
    session: &PipelineSession,
    leann: Option<&LeannIntegration>,
    learning: &mut Option<&mut LearningIntegration>,
    audit: &mut Option<PipelineAuditRun>,
    agents: Vec<AgentInfo>,
) -> Result<Vec<PreparedWaveAgent>> {
    let ordinal_start = session.agent_results.len();
    let mut prepared = Vec::with_capacity(agents.len());
    for (offset, agent) in agents.into_iter().enumerate() {
        let ordinal = ordinal_start + offset;
        tracing::info!(
            agent_key = %agent.key,
            agent_name = %agent.display_name,
            phase = agent.phase,
            model = %agent.model,
            wave_ordinal = ordinal,
            "Executing agent in parallel wave"
        );
        if let Some(audit) = audit.as_mut() {
            audit.record_agent_planned(ordinal, &agent)?;
        }

        let leann_context = leann
            .map(|li| li.search_context(&session.task, &agent.key))
            .unwrap_or_default();
        let agent_session = PipelineSession {
            id: session.id.clone(),
            pipeline_type: session.pipeline_type.clone(),
            task: session.task.clone(),
            started_at: session.started_at,
            agent_results: session.agent_results.clone(),
            leann_context,
        };
        let learning_ctx = learning
            .as_deref_mut()
            .map(|li| {
                li.on_agent_start(
                    &agent.key,
                    &agent.phase.to_string(),
                    &session.task,
                    &session.id,
                )
            })
            .unwrap_or_default();

        let (messages, mut system, tools) = facade
            .build_prompt_for_attempt(&agent_session, &agent, 1)
            .await?;
        append_learning_context(&mut system, &learning_ctx);
        let prompt = if let Some(audit) = audit.as_ref() {
            Some(audit.record_prompt(ordinal, &agent, &messages, &system, &tools)?)
        } else {
            None
        };
        if let Some(audit) = audit.as_ref() {
            audit.record_attempt_started(ordinal, &agent, 1)?;
        }
        prepared.push(PreparedWaveAgent {
            ordinal,
            agent,
            session: agent_session,
            messages,
            system,
            tools,
            prompt,
        });
    }
    Ok(prepared)
}

async fn execute_prepared_wave_agent(
    facade: &dyn PipelineFacade,
    llm: &dyn LlmClient,
    prepared: PreparedWaveAgent,
) -> std::result::Result<
    (PreparedWaveAgent, AgentResult, QualityScore),
    (PreparedWaveAgent, anyhow::Error),
> {
    let start = Instant::now();
    let response = match llm
        .run_agent(AgentExecutionRequest {
            session_id: prepared.session.id.clone(),
            pipeline_type: prepared.session.pipeline_type.clone(),
            task: prepared.session.task.clone(),
            cwd: None,
            ordinal: prepared.ordinal,
            attempt: 1,
            agent: prepared.agent.clone(),
            messages: prepared.messages.clone(),
            system: prepared.system.clone(),
            tools: prepared.tools.clone(),
            allowed_tools: Vec::new(),
        })
        .await
    {
        Ok(response) => response,
        Err(error) => return Err((prepared, error)),
    };
    let mut result = AgentResult {
        output: response.content,
        tool_use_log: response.tool_uses,
        tokens_in: response.tokens_in,
        tokens_out: response.tokens_out,
        cost_usd: 0.0,
        duration: start.elapsed(),
        quality: None,
    };
    let quality = match facade
        .score_quality(&prepared.session, &prepared.agent, &result)
        .await
    {
        Ok(quality) => quality,
        Err(error) => return Err((prepared, error)),
    };
    result.quality = Some(quality.clone());
    Ok((prepared, result, quality))
}

async fn recover_wave_initial_failure(
    facade: &dyn PipelineFacade,
    llm: &dyn LlmClient,
    audit: &mut Option<PipelineAuditRun>,
    prepared: PreparedWaveAgent,
    error: anyhow::Error,
) -> Result<(
    PreparedWaveAgent,
    AgentResult,
    QualityScore,
    usize,
    Option<PromptHashes>,
)> {
    if !is_context_window_error(&error) {
        if let Some(audit) = audit.as_ref() {
            audit.record_attempt_failed(
                prepared.ordinal,
                &prepared.agent,
                1,
                &error.to_string(),
            )?;
        }
        fail_audit(audit, &error.to_string())?;
        return Err(error);
    }
    tracing::warn!(
        agent_key = %prepared.agent.key,
        "pipeline parallel-wave prompt exceeded context; rebuilding with retry budget"
    );
    let mut attempt = 1usize;
    loop {
        if attempt >= PIPELINE_MAX_ATTEMPTS {
            if let Some(audit) = audit.as_ref() {
                audit.record_attempt_failed(
                    prepared.ordinal,
                    &prepared.agent,
                    attempt,
                    &error.to_string(),
                )?;
            }
            fail_audit(audit, &error.to_string())?;
            return Err(error);
        }
        attempt += 1;
        match run_wave_attempt(facade, llm, audit, &prepared, attempt, None).await {
            Ok((result, quality, prompt)) => {
                break Ok((prepared, result, quality, attempt, prompt));
            }
            Err(retry_error)
                if is_context_window_error(&retry_error) && attempt < PIPELINE_MAX_ATTEMPTS =>
            {
                tracing::warn!(
                    agent_key = %prepared.agent.key,
                    attempt = attempt,
                    "pipeline parallel-wave prompt exceeded context; rebuilding with tighter retry budget"
                );
                continue;
            }
            Err(retry_error) => {
                if let Some(audit) = audit.as_ref() {
                    audit.record_attempt_failed(
                        prepared.ordinal,
                        &prepared.agent,
                        attempt,
                        &retry_error.to_string(),
                    )?;
                }
                fail_audit(audit, &retry_error.to_string())?;
                return Err(retry_error);
            }
        }
    }
}

async fn run_wave_attempt(
    facade: &dyn PipelineFacade,
    llm: &dyn LlmClient,
    audit: &Option<PipelineAuditRun>,
    prepared: &PreparedWaveAgent,
    attempt: usize,
    reflexion_section: Option<String>,
) -> Result<(AgentResult, QualityScore, Option<PromptHashes>)> {
    let (messages, mut system, tools) = facade
        .build_prompt_for_attempt(&prepared.session, &prepared.agent, attempt as u8)
        .await?;
    if let Some(section) = reflexion_section {
        system.push(serde_json::json!({
            "text": section,
        }));
        tracing::info!(
            agent_key = %prepared.agent.key,
            attempt = attempt,
            "Reflexion context injected"
        );
    }
    let prompt = if let Some(audit) = audit.as_ref() {
        Some(audit.record_prompt(
            prepared.ordinal,
            &prepared.agent,
            &messages,
            &system,
            &tools,
        )?)
    } else {
        None
    };
    if let Some(audit) = audit.as_ref() {
        audit.record_attempt_started(prepared.ordinal, &prepared.agent, attempt)?;
    }
    let start = Instant::now();
    let response = llm
        .run_agent(AgentExecutionRequest {
            session_id: prepared.session.id.clone(),
            pipeline_type: prepared.session.pipeline_type.clone(),
            task: prepared.session.task.clone(),
            cwd: None,
            ordinal: prepared.ordinal,
            attempt,
            agent: prepared.agent.clone(),
            messages,
            system,
            tools,
            allowed_tools: Vec::new(),
        })
        .await?;
    let mut result = AgentResult {
        output: response.content,
        tool_use_log: response.tool_uses,
        tokens_in: response.tokens_in,
        tokens_out: response.tokens_out,
        cost_usd: 0.0,
        duration: start.elapsed(),
        quality: None,
    };
    let quality = facade
        .score_quality(&prepared.session, &prepared.agent, &result)
        .await?;
    result.quality = Some(quality.clone());
    Ok((result, quality, prompt))
}

#[allow(clippy::too_many_arguments)]
async fn commit_wave_agent_completion(
    facade: &dyn PipelineFacade,
    session: &mut PipelineSession,
    leann: Option<&LeannIntegration>,
    learning: &mut Option<&mut LearningIntegration>,
    audit: &mut Option<PipelineAuditRun>,
    prepared: PreparedWaveAgent,
    result: AgentResult,
    quality: QualityScore,
    attempts: Vec<AgentAttemptRecord>,
    accepted_prompt: Option<PromptHashes>,
) -> Result<()> {
    if let Err(error) = facade
        .process_completion(session, &prepared.agent, &result, &quality)
        .await
    {
        fail_audit(audit, &error.to_string())?;
        return Err(error);
    }
    if let Some(li) = learning.as_deref_mut() {
        li.on_agent_complete(&prepared.agent.key, quality.overall, &result.output);
    }
    reindex_modified_files(leann, &prepared.agent, &result).await;
    if let Some(audit) = audit.as_mut()
        && let Some(prompt) = accepted_prompt
    {
        audit.record_agent_completed(
            prepared.ordinal,
            &prepared.agent,
            &result,
            attempts,
            prompt,
        )?;
    }
    session.agent_results.push((prepared.agent, result));
    Ok(())
}
