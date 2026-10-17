use anyhow::Result;

use super::quality_gate::{
    force_acceptance_reason, has_non_bypassable_quality_failure, quality_gate_acceptance,
};
use super::support::{
    append_learning_context, append_reflexion_context, log_agent_completed, quality_failure_reason,
    record_reflexion_failure, record_research_agent_artifacts, reindex_modified_files,
};
use super::*;
use crate::audit::types::AgentAttemptRecord;
use crate::learning::integration::LearningContext;

struct AgentExecutionOutcome {
    result: AgentResult,
    quality: QualityScore,
    attempts: Vec<AgentAttemptRecord>,
    accepted_prompt: Option<PromptHashes>,
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn run_single_agent(
    facade: &dyn PipelineFacade,
    llm: &dyn LlmClient,
    session: &mut PipelineSession,
    leann: Option<&LeannIntegration>,
    reflexion: &mut Option<&mut ReflexionInjector>,
    learning: &mut Option<&mut LearningIntegration>,
    audit: &mut Option<PipelineAuditRun>,
    agent: AgentInfo,
    options: PipelineRunOptions,
) -> Result<()> {
    let ordinal = session.agent_results.len();
    tracing::info!(
        agent_key = %agent.key,
        agent_name = %agent.display_name,
        phase = agent.phase,
        model = %agent.model,
        "Executing agent"
    );
    if let Some(audit) = audit.as_mut() {
        audit.record_agent_planned(ordinal, &agent)?;
    }

    if let Some(li) = leann {
        session.leann_context = li.search_context(&session.task, &agent.key);
    }

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

    let outcome = run_agent_attempts(
        facade,
        llm,
        session,
        &agent,
        ordinal,
        &learning_ctx,
        reflexion,
        audit,
        options,
    )
    .await?;
    let result = commit_single_agent_completion(
        facade, session, leann, learning, audit, &agent, ordinal, outcome,
    )
    .await?;
    session.agent_results.push((agent, result));
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn run_agent_attempts(
    facade: &dyn PipelineFacade,
    llm: &dyn LlmClient,
    session: &mut PipelineSession,
    agent: &AgentInfo,
    ordinal: usize,
    learning_ctx: &LearningContext,
    reflexion: &mut Option<&mut ReflexionInjector>,
    audit: &mut Option<PipelineAuditRun>,
    options: PipelineRunOptions,
) -> Result<AgentExecutionOutcome> {
    let repair_artifact = agent
        .critical
        .then(|| facade.critical_validator_artifact(agent))
        .flatten();
    let pending_repair = if let (Some(artifact_key), Some(audit_run)) =
        (repair_artifact.as_deref(), audit.as_ref())
    {
        audit_run.pending_artifact_repair(artifact_key, &agent.key)?
    } else {
        None
    };
    if let (Some(artifact_key), Some((pending_attempt, findings))) =
        (repair_artifact.as_deref(), pending_repair)
    {
        if let Err(error) = critical_repair::repair_judged_artifact(
            facade,
            llm,
            session,
            agent,
            artifact_key,
            pending_attempt,
            &findings,
            ordinal,
            audit,
        )
        .await
        {
            if let Some(audit_run) = audit.as_ref() {
                audit_run.store().append_event(
                    &session.id,
                    crate::audit::types::PipelineEvent::ArtifactRepairRejected {
                        agent_key: artifact_key.to_string(),
                        attempt: pending_attempt,
                        reason: format!("repair abandoned on resume; kept previous paper: {error}"),
                    },
                )?;
            }
        }
    }
    let mut attempt = if repair_artifact.is_some() {
        audit
            .as_ref()
            .map(|run| run.last_validator_attempt(&agent.key))
            .transpose()?
            .unwrap_or(0)
    } else {
        0
    };
    let mut attempts = Vec::new();
    loop {
        if agent.critical && attempt >= PIPELINE_MAX_ATTEMPTS {
            let reason = format!(
                "critical validator {} exhausted its {} attempt budget before resume",
                agent.key, PIPELINE_MAX_ATTEMPTS
            );
            critical_repair::write_exhausted_validator_draft(
                facade,
                session,
                agent,
                &format!("validator did not return findings; {reason}"),
                audit,
            )?;
            fail_audit(audit, &reason)?;
            return Err(anyhow::anyhow!(reason));
        }
        attempt += 1;
        let (messages, mut system, tools) = match facade
            .build_prompt_for_attempt(session, agent, attempt as u8)
            .await
        {
            Ok(prompt) => prompt,
            Err(error) => {
                if agent.critical && attempt >= PIPELINE_MAX_ATTEMPTS {
                    critical_repair::write_exhausted_validator_draft(
                        facade,
                        session,
                        agent,
                        &format!("validator did not return findings; prompt build failed: {error}"),
                        audit,
                    )?;
                }
                fail_audit(audit, &error.to_string())?;
                return Err(error);
            }
        };
        if attempt == 1 {
            append_learning_context(&mut system, learning_ctx);
        }
        append_reflexion_context(&mut system, reflexion, agent, attempt);

        let prompt_hashes = if let Some(audit) = audit.as_ref() {
            Some(audit.record_prompt(ordinal, agent, &messages, &system, &tools)?)
        } else {
            None
        };
        if let Some(audit) = audit.as_ref() {
            audit.record_attempt_started(ordinal, agent, attempt)?;
        }

        let agent_start = Instant::now();
        let llm_response = match llm
            .run_agent(AgentExecutionRequest {
                session_id: session.id.clone(),
                pipeline_type: session.pipeline_type.clone(),
                task: session.task.clone(),
                cwd: None,
                ordinal,
                attempt,
                agent: agent.clone(),
                messages,
                system,
                tools,
                allowed_tools: Vec::new(),
                timeout_secs: None,
                disable_auto_background: false,
                read_roots: Vec::new(),
                write_roots: Vec::new(),
                provider_env_resolution: None,
            })
            .await
        {
            Ok(response) => response,
            Err(error) => {
                if agent.critical && attempt >= PIPELINE_MAX_ATTEMPTS {
                    critical_repair::write_exhausted_validator_draft(
                        facade,
                        session,
                        agent,
                        &format!("validator did not return findings; provider failed: {error}"),
                        audit,
                    )?;
                }
                if should_retry_single_agent_transport(audit, agent, ordinal, attempt, &error)
                    .await?
                {
                    continue;
                }
                if let Some(audit) = audit.as_ref() {
                    audit.record_attempt_failed(ordinal, agent, attempt, &error.to_string())?;
                }
                fail_audit(audit, &error.to_string())?;
                return Err(error);
            }
        };

        let mut result = AgentResult {
            output: llm_response.content,
            tool_use_log: llm_response.tool_uses,
            tokens_in: llm_response.tokens_in,
            tokens_out: llm_response.tokens_out,
            cost_usd: 0.0,
            duration: agent_start.elapsed(),
            quality: None,
        };
        let quality = match facade.score_quality(session, agent, &result).await {
            Ok(quality) => quality,
            Err(error) => {
                if agent.critical && attempt >= PIPELINE_MAX_ATTEMPTS {
                    critical_repair::write_exhausted_validator_draft(
                        facade,
                        session,
                        agent,
                        &format!(
                            "validator did not return findings; quality scoring failed: {error}"
                        ),
                        audit,
                    )?;
                }
                fail_audit(audit, &error.to_string())?;
                return Err(error);
            }
        };
        result.quality = Some(quality.clone());
        let meets_threshold = quality.overall >= agent.quality_threshold;
        let effective_options = if has_non_bypassable_quality_failure(&quality) {
            PipelineRunOptions::default()
        } else {
            options
        };
        let gate = quality_gate_acceptance(meets_threshold, agent, attempt, effective_options);
        let failure_reason =
            (!meets_threshold).then(|| quality_failure_reason(agent, quality.overall));
        let failure_reason = if gate.force_accepted {
            Some(force_acceptance_reason(failure_reason))
        } else {
            failure_reason
        };
        if let Some(audit) = audit.as_ref() {
            if let Some(artifact_key) = repair_artifact.as_ref()
                && !gate.accepted
                && attempt < PIPELINE_MAX_ATTEMPTS
            {
                audit.record_validator_repair_pending(
                    &agent.key,
                    artifact_key,
                    attempt,
                    &result.output,
                )?;
            } else if repair_artifact.is_some() {
                audit.record_validator_progress(&agent.key, attempt)?;
            }
        }
        if let Some(audit) = audit.as_ref() {
            attempts.push(audit.record_attempt_completed(
                ordinal,
                agent,
                attempt,
                &result,
                gate.accepted,
                failure_reason.clone(),
            )?);
            if gate.force_accepted {
                audit.record_quality_gate_force_accepted(
                    ordinal,
                    agent,
                    attempt,
                    quality.overall,
                    failure_reason.as_deref().unwrap_or("force-accepted"),
                )?;
            }
        }
        log_agent_completed(agent, attempt, &quality, &result);

        if gate.accepted {
            return Ok(AgentExecutionOutcome {
                result,
                quality,
                attempts,
                accepted_prompt: prompt_hashes,
            });
        }
        if attempt >= PIPELINE_MAX_ATTEMPTS && agent.critical {
            let reason = quality_gate_failure(agent, quality.overall, attempt);
            critical_repair::write_exhausted_validator_draft(
                facade,
                session,
                agent,
                &format!(
                    "{}\n\nLast validator findings:\n{}",
                    quality_gate_failure(agent, quality.overall, attempt),
                    result.output
                ),
                audit,
            )?;
            fail_audit(audit, &reason)?;
            return Err(anyhow::anyhow!(reason));
        }

        record_reflexion_failure(
            reflexion,
            agent,
            attempt,
            &result,
            &quality,
            &failure_reason,
        );
        if let Some(audit) = audit.as_ref() {
            audit.record_agent_retry(
                ordinal,
                agent,
                attempt,
                failure_reason
                    .as_deref()
                    .unwrap_or("quality threshold not met"),
            )?;
        }
        if let Some(artifact_key) = repair_artifact.as_ref() {
            if let Err(error) = critical_repair::repair_judged_artifact(
                facade,
                llm,
                session,
                agent,
                &artifact_key,
                attempt,
                &result.output,
                ordinal,
                audit,
            )
            .await
            {
                if let Some(audit) = audit.as_ref() {
                    audit.store().append_event(
                        &session.id,
                        crate::audit::types::PipelineEvent::ArtifactRepairRejected {
                            agent_key: artifact_key.clone(),
                            attempt,
                            reason: format!("repair failed; kept previous paper: {error}"),
                        },
                    )?;
                }
            }
        }
    }
}

pub(super) async fn should_retry_single_agent_transport(
    audit: &Option<PipelineAuditRun>,
    agent: &AgentInfo,
    ordinal: usize,
    attempt: usize,
    error: &anyhow::Error,
) -> Result<bool> {
    if is_context_window_error(error) && attempt < PIPELINE_MAX_ATTEMPTS {
        tracing::warn!(
            agent_key = %agent.key,
            "pipeline prompt exceeded context; rebuilding with retry budget"
        );
        return Ok(true);
    }
    if is_retryable_pipeline_attempt_error(error) && attempt < PIPELINE_MAX_ATTEMPTS {
        let reason = error.to_string();
        if let Some(audit) = audit.as_ref() {
            audit.record_attempt_failed(ordinal, agent, attempt, &reason)?;
            audit.record_agent_retry(ordinal, agent, attempt, &reason)?;
        }
        let delay = pipeline_attempt_retry_delay(attempt, error);
        tracing::warn!(
            agent_key = %agent.key,
            attempt = attempt,
            retry_delay_ms = delay.as_millis() as u64,
            error = %reason,
            "pipeline agent attempt failed with retryable transport error"
        );
        sleep_before_pipeline_retry(delay, tokio::time::sleep).await;
        return Ok(true);
    }
    Ok(false)
}

#[allow(clippy::too_many_arguments)]
async fn commit_single_agent_completion(
    facade: &dyn PipelineFacade,
    session: &mut PipelineSession,
    leann: Option<&LeannIntegration>,
    learning: &mut Option<&mut LearningIntegration>,
    audit: &mut Option<PipelineAuditRun>,
    agent: &AgentInfo,
    ordinal: usize,
    outcome: AgentExecutionOutcome,
) -> Result<AgentResult> {
    let AgentExecutionOutcome {
        result,
        quality,
        attempts,
        accepted_prompt,
    } = outcome;

    if let Err(error) = facade
        .process_completion(session, agent, &result, &quality)
        .await
    {
        fail_audit(audit, &error.to_string())?;
        return Err(error);
    }
    if let Some(li) = learning.as_deref_mut() {
        li.on_agent_complete(&agent.key, quality.overall, &result.output);
    }
    reindex_modified_files(leann, agent, &result).await;

    if let Some(audit) = audit.as_mut()
        && let Some(prompt) = accepted_prompt
    {
        audit.record_agent_completed(ordinal, agent, &result, attempts, prompt)?;
        if session.pipeline_type == PipelineType::Research {
            record_research_agent_artifacts(audit, &session.id, ordinal, agent, &result)?;
        }
    }
    Ok(result)
}
