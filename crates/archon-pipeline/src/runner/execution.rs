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
                run_single_agent(
                    facade, llm, session, leann, reflexion, learning, &mut audit, agent, options,
                )
                .await?;
            }
            NextAgent::ContinueWave(agents) => {
                run_parallel_wave(
                    facade, llm, session, leann, reflexion, learning, &mut audit, agents, options,
                )
                .await?;
            }
            NextAgent::Skip(reason) => {
                tracing::warn!(reason = %reason, "Skipping agent");
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
    Ok(result)
}

fn relative_to_bundle(bundle_dir: &Path, path: &Path) -> String {
    path.strip_prefix(bundle_dir)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn is_context_window_error(error: &anyhow::Error) -> bool {
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

fn is_retryable_pipeline_attempt_error(error: &anyhow::Error) -> bool {
    if let Some(err) = error.downcast_ref::<archon_llm::provider::LlmError>() {
        if err.is_context_window_exceeded() {
            return false;
        }
        return matches!(
            err,
            archon_llm::provider::LlmError::Http(_)
                | archon_llm::provider::LlmError::RateLimited { .. }
                | archon_llm::provider::LlmError::Overloaded
                | archon_llm::provider::LlmError::Server { .. }
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
        "server overloaded",
        "rate limited",
        "429",
        "500",
        "502",
        "503",
        "504",
    ]
    .iter()
    .any(|marker| message.contains(marker))
}

fn pipeline_attempt_retry_delay(attempt: usize) -> Duration {
    let exponent = (attempt as u32).saturating_sub(1);
    let delay_ms = 250u64.saturating_mul(2u64.saturating_pow(exponent));
    Duration::from_millis(delay_ms.min(2_000))
}

fn quality_gate_failure(agent: &AgentInfo, score: f64, attempt: usize) -> String {
    format!(
        "Critical agent '{}' failed quality threshold {:.2} after {} attempts (best score: {:.2})",
        agent.key, agent.quality_threshold, attempt, score
    )
}

fn fail_audit(audit: &mut Option<PipelineAuditRun>, error: &str) -> Result<()> {
    if let Some(audit) = audit.as_mut() {
        audit.fail(error)?;
    }
    Ok(())
}
