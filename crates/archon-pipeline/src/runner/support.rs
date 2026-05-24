use anyhow::Result;

use super::*;
use crate::learning::integration::LearningContext;
use crate::learning::reflexion::{FailedTrajectory, ReflexionInjector};
use crate::research::artifacts::write_research_agent_artifacts;

pub(super) fn append_learning_context(
    system: &mut Vec<serde_json::Value>,
    learning_ctx: &LearningContext,
) {
    if !learning_ctx.sona_context.is_empty() {
        system.push(serde_json::json!({
            "text": format!("## SONA Trajectory\n{}", learning_ctx.sona_context),
        }));
    }
    if !learning_ctx.reasoning_context.is_empty() {
        system.push(serde_json::json!({
            "text": format!("## Reasoning Context\n{}", learning_ctx.reasoning_context),
        }));
    }
    if !learning_ctx.desc_episodes.is_empty() {
        system.push(serde_json::json!({
            "text": format!("## DESC Episodes\n{}", learning_ctx.desc_episodes.join("\n\n")),
        }));
    }
}

pub(super) fn append_reflexion_context(
    system: &mut Vec<serde_json::Value>,
    reflexion: &mut Option<&mut ReflexionInjector>,
    agent: &AgentInfo,
    attempt: usize,
) {
    if attempt > 1
        && let Some(ri) = reflexion.as_deref()
        && let Some(ctx) = ri.inject_reflexion(&agent.key)
    {
        system.push(serde_json::json!({
            "text": ctx.formatted_prompt_section,
        }));
        tracing::info!(
            agent_key = %agent.key,
            attempt = attempt,
            "Reflexion context injected"
        );
    }
}

pub(super) fn record_reflexion_failure(
    reflexion: &mut Option<&mut ReflexionInjector>,
    agent: &AgentInfo,
    attempt: usize,
    result: &AgentResult,
    quality: &QualityScore,
    failure_reason: &Option<String>,
) {
    if let Some(ri) = reflexion.as_deref_mut() {
        ri.record_failure(FailedTrajectory {
            agent_name: agent.key.clone(),
            attempt,
            output_summary: result.output.clone(),
            failure_reason: failure_reason
                .clone()
                .unwrap_or_else(|| quality_failure_reason(agent, quality.overall)),
            quality_score: quality.overall,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        });
        tracing::info!(
            agent_key = %agent.key,
            attempt = attempt,
            "Recorded failure for reflexion"
        );
    }
}

pub(super) async fn reindex_modified_files(
    leann: Option<&LeannIntegration>,
    agent: &AgentInfo,
    result: &AgentResult,
) {
    if agent.phase < 4 {
        return;
    }
    if let Some(li) = leann {
        match li.index_modified_files(&result.tool_use_log).await {
            Ok(count) if count > 0 => {
                tracing::info!(
                    agent_key = %agent.key,
                    files_indexed = count,
                    "LEANN re-indexed modified files"
                );
            }
            Err(e) => {
                tracing::warn!(
                    agent_key = %agent.key,
                    error = %e,
                    "LEANN re-indexing failed; continuing"
                );
            }
            _ => {}
        }
    }
}

pub(super) fn record_research_agent_artifacts(
    audit: &PipelineAuditRun,
    session_id: &str,
    ordinal: usize,
    agent: &AgentInfo,
    result: &AgentResult,
) -> Result<()> {
    let bundle_dir = audit.store().bundle_dir(session_id);
    for artifact in
        write_research_agent_artifacts(&bundle_dir, ordinal, &agent.key, &result.output)?
    {
        audit.store().append_event(
            session_id,
            PipelineEvent::ArtifactWritten {
                artifact_type: "research-agent-output".to_string(),
                path: super::relative_to_bundle(&bundle_dir, &artifact.path),
                content_hash: artifact.hash,
            },
        )?;
    }
    Ok(())
}

pub(super) fn quality_failure_reason(agent: &AgentInfo, score: f64) -> String {
    format!(
        "Quality {:.2} below threshold {:.2}",
        score, agent.quality_threshold
    )
}

pub(super) fn log_agent_completed(
    agent: &AgentInfo,
    attempt: usize,
    quality: &QualityScore,
    result: &AgentResult,
) {
    tracing::info!(
        agent_key = %agent.key,
        attempt = attempt,
        quality_overall = quality.overall,
        threshold = agent.quality_threshold,
        meets_threshold = quality.overall >= agent.quality_threshold,
        tokens_in = result.tokens_in,
        tokens_out = result.tokens_out,
        duration_ms = result.duration.as_millis() as u64,
        "Agent completed"
    );
}

pub(super) fn log_wave_agent_completed(
    agent: &AgentInfo,
    attempt: usize,
    quality: &QualityScore,
    result: &AgentResult,
) {
    tracing::info!(
        agent_key = %agent.key,
        attempt = attempt,
        quality_overall = quality.overall,
        threshold = agent.quality_threshold,
        meets_threshold = quality.overall >= agent.quality_threshold,
        tokens_in = result.tokens_in,
        tokens_out = result.tokens_out,
        duration_ms = result.duration.as_millis() as u64,
        "Parallel-wave agent completed"
    );
}
