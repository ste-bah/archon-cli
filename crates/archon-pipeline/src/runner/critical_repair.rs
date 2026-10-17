use super::*;
use anyhow::Result;

pub(super) async fn repair_judged_artifact(
    facade: &dyn PipelineFacade,
    llm: &dyn LlmClient,
    session: &mut PipelineSession,
    validator: &AgentInfo,
    artifact_key: &str,
    attempt: usize,
    findings: &str,
    ordinal: usize,
    audit: &mut Option<PipelineAuditRun>,
) -> Result<()> {
    let Some((artifact_agent, _artifact)) = session
        .agent_results
        .iter()
        .find(|(agent, _)| agent.key == artifact_key)
    else {
        anyhow::bail!("critical validator artifact {artifact_key} is missing from session");
    };
    let current = facade
        .critical_validator_artifact_text(session, validator)
        .ok_or_else(|| {
            anyhow::anyhow!("critical validator artifact {artifact_key} has no judged text")
        })?;
    let artifact_agent = artifact_agent.clone();
    let prompt = facade.build_critical_repair_prompt(session, validator, &current, findings)?;
    let repair_agent = artifact_agent.clone();
    let (_, system, tools) = facade
        .build_prompt_for_attempt(session, &artifact_agent, attempt as u8)
        .await?;
    if let Some(audit) = audit.as_ref() {
        audit.store().append_event(
            &session.id,
            crate::audit::types::PipelineEvent::LlmAttemptStarted {
                ordinal,
                agent_key: artifact_key.to_string(),
                attempt,
                model: repair_agent.model.clone(),
            },
        )?;
    }
    let response = match llm
        .run_agent(AgentExecutionRequest {
            session_id: session.id.clone(),
            pipeline_type: session.pipeline_type.clone(),
            task: session.task.clone(),
            cwd: None,
            ordinal,
            attempt,
            agent: repair_agent,
            messages: vec![serde_json::json!({"role":"user", "content":prompt})],
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
            if let Some(audit) = audit.as_ref() {
                audit.store().append_event(
                    &session.id,
                    crate::audit::types::PipelineEvent::LlmAttemptFailed {
                        ordinal,
                        agent_key: artifact_key.to_string(),
                        attempt,
                        error: error.to_string(),
                    },
                )?;
            }
            return Err(error);
        }
    };
    let repaired = response.content.trim();
    let repaired_result = AgentResult {
        output: repaired.to_string(),
        tool_use_log: Vec::new(),
        tokens_in: response.tokens_in,
        tokens_out: response.tokens_out,
        cost_usd: 0.0,
        duration: std::time::Duration::ZERO,
        quality: None,
    };
    let quality = facade
        .score_quality(session, &artifact_agent, &repaired_result)
        .await?;
    let rejection =
        repair_rejection_reason(repaired, quality.overall, artifact_agent.quality_threshold);
    if let Some(reason) = rejection {
        if let Some(audit) = audit.as_ref() {
            audit.store().append_event(
                &session.id,
                crate::audit::types::PipelineEvent::ArtifactRepairRejected {
                    agent_key: artifact_key.to_string(),
                    attempt,
                    reason,
                },
            )?;
        }
        return Ok(());
    }
    let output_hash = crate::audit::store::sha256_hex(repaired.as_bytes());
    let input_hash = crate::audit::store::sha256_hex(current.as_bytes());
    if let Some(audit) = audit.as_ref() {
        audit.store().append_event(
            &session.id,
            crate::audit::types::PipelineEvent::LlmAttemptCompleted {
                ordinal,
                agent_key: artifact_key.to_string(),
                attempt,
                output_hash: output_hash.clone(),
                tokens_in: response.tokens_in,
                tokens_out: response.tokens_out,
                duration_ms: 0,
            },
        )?;
    }
    if let Some(audit) = audit.as_ref() {
        audit.persist_artifact_repair(
            artifact_key,
            attempt,
            ordinal,
            input_hash,
            output_hash,
            repaired,
        )?;
    }
    let Some((_, artifact)) = session
        .agent_results
        .iter_mut()
        .find(|(agent, _)| agent.key == artifact_key)
    else {
        anyhow::bail!("critical validator artifact {artifact_key} disappeared during repair");
    };
    artifact.output = repaired.to_string();
    if let Some(quality) = artifact.quality.as_mut() {
        quality
            .dimensions
            .insert("artifact_repaired".to_string(), 1.0);
    }
    Ok(())
}

fn repair_rejection_reason(
    repaired: &str,
    quality_score: f64,
    quality_threshold: f64,
) -> Option<String> {
    if repaired.trim().is_empty() {
        Some("repair rejected: response was empty".to_string())
    } else if quality_score < quality_threshold {
        Some("repair rejected: artifact did not meet its quality threshold".to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod repair_validation_tests {
    use super::repair_rejection_reason;

    #[test]
    fn empty_repair_is_rejected() {
        assert_eq!(
            repair_rejection_reason(" \n ", 0.0, 0.5).as_deref(),
            Some("repair rejected: response was empty")
        );
    }

    #[test]
    fn repair_must_pass_basic_artifact_checks() {
        let refusal =
            crate::research::final_steps::score_combiner_output("I cannot help with that request.");
        let valid = crate::research::final_steps::score_combiner_output(
            "# Paper\n\n## Abstract\n\nSummary.\n\n## Introduction\n\nContext.\n\n## Methods\n\nApproach.\n\n## Results\n\nFindings.\n\n## Discussion\n\nAnalysis.\n\n## References\n\nSmith (2024).\n\n## Appendix\n\nDetails.",
        );
        assert!(refusal.overall < 0.5);
        assert!(valid.overall >= 0.5);
        assert!(
            repair_rejection_reason("I cannot help with that request.", refusal.overall, 0.5)
                .is_some()
        );
        assert!(repair_rejection_reason("valid repaired paper", valid.overall, 0.5).is_none());
    }
}

pub(super) fn write_exhausted_validator_draft(
    facade: &dyn PipelineFacade,
    session: &PipelineSession,
    validator: &AgentInfo,
    findings: &str,
    audit: &Option<PipelineAuditRun>,
) -> Result<()> {
    if facade.critical_validator_artifact(validator).is_none() {
        return Ok(());
    }
    let Some(paper) = facade.critical_validator_artifact_text(session, validator) else {
        return Ok(());
    };
    let Some(audit) = audit.as_ref() else {
        return Ok(());
    };
    let bundle_dir = audit.store().bundle_dir(&session.id);
    let findings = if findings.trim().is_empty() {
        "The validator did not return findings."
    } else {
        findings
    };
    for (artifact_type, path, content_hash) in facade
        .write_failed_critical_draft(session, &paper, findings, &bundle_dir)?
        .unwrap_or_default()
    {
        audit.store().append_event(
            &session.id,
            crate::audit::types::PipelineEvent::ArtifactWritten {
                artifact_type,
                path: path
                    .strip_prefix(&bundle_dir)?
                    .to_string_lossy()
                    .into_owned(),
                content_hash,
            },
        )?;
    }
    Ok(())
}
