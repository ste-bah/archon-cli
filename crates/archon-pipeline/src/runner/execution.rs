use anyhow::Context;

use super::next_agent::{SkipGuard, StepProgress};
use super::*;

// ---------------------------------------------------------------------------
// Runner loop
// ---------------------------------------------------------------------------

/// Execute a full pipeline run.
///
/// The runner repeatedly asks the facade for the next agent, builds a fresh
/// prompt (context isolation), sends it to the LLM, scores quality, and
/// records the result. Once the facade signals [`NextAgent::Done`], it
/// finalizes and returns the [`PipelineResult`].
pub async fn run_pipeline(
    facade: &dyn PipelineFacade,
    llm: &dyn LlmClient,
    task: &str,
    leann: Option<&LeannIntegration>,
    mut reflexion: Option<&mut ReflexionInjector>,
    mut learning: Option<&mut LearningIntegration>,
) -> Result<PipelineResult> {
    let mut session = facade.init_session(task).await?;
    run_pipeline_inner(
        facade,
        llm,
        &mut session,
        leann,
        (&mut reflexion, &mut learning),
        None,
        PipelineRunOptions::default(),
    )
    .await
}

/// Execute a built-in pipeline with a durable audited bundle.
pub async fn run_pipeline_audited(
    facade: &dyn PipelineFacade,
    llm: &dyn LlmClient,
    task: &str,
    worktree: &Path,
    leann: Option<&LeannIntegration>,
    mut reflexion: Option<&mut ReflexionInjector>,
    mut learning: Option<&mut LearningIntegration>,
) -> Result<PipelineResult> {
    let mut session = facade.init_session(task).await?;
    let pipeline_type = session.pipeline_type.clone();
    let audit = PipelineAuditRun::start(worktree, &session.id, pipeline_type, &session.task)?;
    run_pipeline_inner(
        facade,
        llm,
        &mut session,
        leann,
        (&mut reflexion, &mut learning),
        Some(audit),
        PipelineRunOptions::default(),
    )
    .await
}

/// Resume a verified audited bundle without repeating completed agents.
pub async fn resume_pipeline_audited(
    facade: &dyn PipelineFacade,
    llm: &dyn LlmClient,
    session_id: &str,
    worktree: &Path,
    leann: Option<&LeannIntegration>,
    reflexion: Option<&mut ReflexionInjector>,
    learning: Option<&mut LearningIntegration>,
) -> Result<PipelineResult> {
    resume_pipeline_audited_with_options(
        facade,
        llm,
        session_id,
        worktree,
        leann,
        (reflexion, learning),
        PipelineRunOptions::default(),
    )
    .await
}

/// Resume a verified audited bundle with explicit runner options.
pub async fn resume_pipeline_audited_with_options(
    facade: &dyn PipelineFacade,
    llm: &dyn LlmClient,
    session_id: &str,
    worktree: &Path,
    leann: Option<&LeannIntegration>,
    (mut reflexion, mut learning): (
        Option<&mut ReflexionInjector>,
        Option<&mut LearningIntegration>,
    ),
    options: PipelineRunOptions,
) -> Result<PipelineResult> {
    let audit = PipelineAuditRun::resume(worktree, session_id)?;
    let mut session = facade.init_session(&audit.state().task).await?;
    session.id = audit.state().session_id.clone();
    session.pipeline_type = audit.state().pipeline_type.clone();
    session.agent_results = audit.hydrate_results()?;
    run_pipeline_inner(
        facade,
        llm,
        &mut session,
        leann,
        (&mut reflexion, &mut learning),
        Some(audit),
        options,
    )
    .await
}

async fn run_pipeline_inner(
    facade: &dyn PipelineFacade,
    llm: &dyn LlmClient,
    session: &mut PipelineSession,
    leann: Option<&LeannIntegration>,
    (reflexion, learning): (
        &mut Option<&mut ReflexionInjector>,
        &mut Option<&mut LearningIntegration>,
    ),
    mut audit: Option<PipelineAuditRun>,
    options: PipelineRunOptions,
) -> Result<PipelineResult> {
    tracing::info!(
        session_id = %session.id,
        pipeline_type = ?session.pipeline_type,
        task = %session.task,
        leann_enabled = leann.is_some(),
        "Pipeline session initialised"
    );

    let mut skips = SkipGuard::default();
    loop {
        let next = match facade.next_agent(session).await {
            Ok(next) => next,
            Err(error) => {
                fail_audit(&mut audit, &error.to_string())?;
                return Err(error);
            }
        };
        match next {
            NextAgent::Continue(agent) => {
                let step = StepProgress::start(session, "Continue", std::slice::from_ref(&agent));
                run_single_agent(
                    facade, llm, session, leann, reflexion, learning, &mut audit, agent, options,
                )
                .await?;
                finish_step(step, session, &mut audit)?;
                skips.reset();
            }
            NextAgent::ContinueWave(agents) => {
                let step = StepProgress::start(session, "ContinueWave", &agents);
                run_parallel_wave(
                    facade, llm, session, leann, reflexion, learning, &mut audit, agents, options,
                )
                .await?;
                finish_step(step, session, &mut audit)?;
                skips.reset();
            }
            NextAgent::Skip(reason) => {
                tracing::warn!(reason = %reason, "Skipping agent");
                if let Err(error) = skips.record(session, &reason) {
                    fail_audit(&mut audit, &error.to_string())?;
                    return Err(error);
                }
            }
            NextAgent::Done => {
                tracing::info!(
                    session_id = %session.id,
                    agents_executed = session.agent_results.len(),
                    "Pipeline loop complete"
                );
                break;
            }
        }
    }

    let session_id = session.id.clone();
    let pipeline_type = session.pipeline_type.clone();
    let placeholder = PipelineSession {
        id: session_id,
        pipeline_type,
        task: String::new(),
        started_at: Instant::now(),
        agent_results: Vec::new(),
        leann_context: String::new(),
    };
    let owned_session = std::mem::replace(session, placeholder);
    let result = match facade.finalize(owned_session).await {
        Ok(result) => result,
        Err(error) => {
            fail_audit(&mut audit, &error.to_string())?;
            return Err(error);
        }
    };
    if result.pipeline_type == PipelineType::Research
        && let Some(audit_run) = audit.as_ref()
    {
        let bundle_dir = audit_run.store().bundle_dir(&result.session_id);
        match write_final_research_artifacts(&bundle_dir, &result.final_output) {
            Ok(artifacts) => {
                let markdown_path = relative_to_bundle(&bundle_dir, &artifacts.markdown_path);
                let pdf_path = relative_to_bundle(&bundle_dir, &artifacts.pdf_path);
                audit_run.store().append_event(
                    &result.session_id,
                    PipelineEvent::ArtifactWritten {
                        artifact_type: "research-paper-markdown".to_string(),
                        path: markdown_path,
                        content_hash: artifacts.markdown_hash,
                    },
                )?;
                audit_run.store().append_event(
                    &result.session_id,
                    PipelineEvent::ArtifactWritten {
                        artifact_type: "research-paper-pdf".to_string(),
                        path: pdf_path,
                        content_hash: artifacts.pdf_hash,
                    },
                )?;
            }
            Err(error) => {
                let error = error.context("failed to produce final APA research paper artifacts");
                fail_audit(&mut audit, &error.to_string())?;
                return Err(error);
            }
        }
    }
    if let Some(audit) = audit.as_mut() {
        audit.complete(&result.final_output)?;
    }
    if let Some(audit_run) = audit.as_ref() {
        let bundle_dir = audit_run.store().bundle_dir(&result.session_id);
        let events = facade
            .after_run_completed(&bundle_dir)
            .await
            .context("run completed, but post-completion artifact handling failed")?;
        for (artifact_type, path, content_hash) in events {
            if let Err(error) = audit_run.store().append_event(
                &result.session_id,
                PipelineEvent::ArtifactWritten {
                    artifact_type,
                    path: relative_to_bundle(&bundle_dir, &path),
                    content_hash,
                },
            ) {
                tracing::error!(
                    session_id = %result.session_id,
                    error = %error,
                    "run completed and artifacts were updated, but recording post-completion artifact event failed"
                );
                return Err(error).context(
                    "run completed and artifacts were updated, but recording post-completion artifact event failed",
                );
            }
        }
    }
    Ok(result)
}

/// Fail the run, and its audit, when a step added the wrong number of results.
fn finish_step(
    step: StepProgress,
    session: &PipelineSession,
    audit: &mut Option<PipelineAuditRun>,
) -> Result<()> {
    if let Err(error) = step.check(session) {
        fail_audit(audit, &error.to_string())?;
        return Err(error);
    }
    Ok(())
}

pub(super) fn relative_to_bundle(bundle_dir: &Path, path: &Path) -> String {
    path.strip_prefix(bundle_dir)
        .unwrap_or(path)
        .display()
        .to_string()
}

pub(super) fn is_context_window_error(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<archon_llm::provider::LlmError>()
        .is_some_and(|err| err.is_context_window_exceeded())
        || archon_llm::context_window::classify_context_window_error(
            None,
            None,
            None,
            &error.to_string(),
            Some("pipeline"),
            None,
        )
        .is_some()
}

pub(super) fn is_retryable_pipeline_attempt_error(error: &anyhow::Error) -> bool {
    if let Some(err) = error.downcast_ref::<archon_llm::provider::LlmError>() {
        if err.is_context_window_exceeded() {
            return false;
        }
        return matches!(
            err,
            archon_llm::provider::LlmError::Http(_)
                | archon_llm::provider::LlmError::RateLimited { .. }
                | archon_llm::provider::LlmError::Overloaded { .. }
                | archon_llm::provider::LlmError::Server { status: 429, .. }
                | archon_llm::provider::LlmError::Server {
                    status: 500..=599,
                    ..
                }
        );
    }

    let message = error.to_string().to_ascii_lowercase();
    [
        "http error",
        "error decoding response body",
        "connection reset",
        "connection closed",
        "connection refused",
        "timed out",
        "timeout",
        "temporarily unavailable",
        "500",
        "502",
        "503",
        "504",
    ]
    .iter()
    .any(|marker| message.contains(marker))
        || has_pipeline_overload_error_code(&message)
}

const PIPELINE_OVERLOAD_BACKOFF_CAP: Duration = Duration::from_secs(300); // Keep provider overload waits within five minutes.

pub(super) fn pipeline_attempt_retry_delay(attempt: usize, error: &anyhow::Error) -> Duration {
    if let Some(retry_after) = provider_retry_after(error) {
        return retry_after.min(PIPELINE_OVERLOAD_BACKOFF_CAP);
    }
    if is_pipeline_overload_error(error) {
        let exponent = (attempt as u32).saturating_sub(1);
        let delay = Duration::from_secs(10u64.saturating_mul(2u64.saturating_pow(exponent)));
        return delay.min(PIPELINE_OVERLOAD_BACKOFF_CAP);
    }
    let exponent = (attempt as u32).saturating_sub(1);
    let delay_ms = 250u64.saturating_mul(2u64.saturating_pow(exponent));
    Duration::from_millis(delay_ms.min(2_000))
}

fn provider_retry_after(error: &anyhow::Error) -> Option<Duration> {
    use archon_llm::provider::LlmError;

    if let Some(LlmError::Server {
        status: 529,
        pipeline_retry_after_secs: Some(retry_after_secs),
        ..
    }) = error.downcast_ref::<LlmError>()
    {
        return Some(Duration::from_secs(*retry_after_secs).min(PIPELINE_OVERLOAD_BACKOFF_CAP));
    }
    if let Some(LlmError::Overloaded {
        status,
        retry_after_secs: Some(retry_after_secs),
    }) = error.downcast_ref::<LlmError>()
    {
        if matches!(status, None | Some(429 | 529)) {
            return Some(Duration::from_secs(*retry_after_secs).min(PIPELINE_OVERLOAD_BACKOFF_CAP));
        }
    }
    if let Some(LlmError::RateLimited {
        pipeline_retry_after_secs: Some(retry_after_secs),
        ..
    }) = error.downcast_ref::<LlmError>()
    {
        return Some(Duration::from_secs(*retry_after_secs).min(PIPELINE_OVERLOAD_BACKOFF_CAP));
    }
    if let Some(LlmError::RateLimited {
        retry_after_secs,
        from_provider: true,
        ..
    }) = error.downcast_ref::<LlmError>()
    {
        return Some(Duration::from_secs(*retry_after_secs).min(PIPELINE_OVERLOAD_BACKOFF_CAP));
    }
    None
}

fn is_pipeline_overload_error(error: &anyhow::Error) -> bool {
    use archon_llm::provider::LlmError;

    if let Some(err) = error.downcast_ref::<LlmError>() {
        return matches!(err, LlmError::RateLimited { .. })
            || matches!(
                err,
                LlmError::Overloaded {
                    status: None | Some(429 | 529),
                    ..
                }
            )
            || matches!(
                err,
                LlmError::Server {
                    status: 429 | 529,
                    ..
                }
            );
    }
    let message = error.to_string().to_ascii_lowercase();
    has_pipeline_overload_error_code(&message)
}

fn has_pipeline_overload_error_code(message: &str) -> bool {
    [
        "overloaded_error",
        "server_is_overloaded",
        "rate_limit_error",
    ]
    .iter()
    .any(|code| {
        message.match_indices(code).any(|(index, _)| {
            let before = message[..index].chars().next_back();
            let after = message[index + code.len()..].chars().next();
            before.is_none_or(|ch| !ch.is_ascii_alphanumeric() && ch != '_')
                && after.is_none_or(|ch| !ch.is_ascii_alphanumeric() && ch != '_')
        })
    })
}

pub(super) fn quality_gate_failure(agent: &AgentInfo, score: f64, attempt: usize) -> String {
    format!(
        "Critical agent '{}' failed quality threshold {:.2} after {} attempts (best score: {:.2})",
        agent.key, agent.quality_threshold, attempt, score
    )
}

pub(super) fn fail_audit(audit: &mut Option<PipelineAuditRun>, error: &str) -> Result<()> {
    if let Some(audit) = audit.as_mut() {
        audit.fail(error)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "execution_tests.rs"]
mod retry_delay_tests;
