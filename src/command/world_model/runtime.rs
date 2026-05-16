use std::path::PathBuf;

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::Serialize;

use archon_world_model::counterfactual::{CounterfactualExample, CounterfactualScore};
use archon_world_model::embedding::{EmbeddingRequest, WorldEmbeddingAdapter};
use archon_world_model::schema::WorldTraceRow;
use archon_world_model::storage::WorldModelStore;

pub(crate) fn record_runtime_advisory(
    config: &archon_core::config::ArchonConfig,
    surface: archon_world_model::integration::WorldAdvisorSurface,
    session_id: &str,
    action_ref: &str,
    summary: &str,
) -> archon_world_model::integration::WorldAdvisorSurfaceRecord {
    let record = runtime_advisory_record(config, surface, session_id, action_ref, summary)
        .unwrap_or_else(|_| {
            archon_world_model::integration::WorldAdvisorSurfaceRecord::unavailable(
                surface,
                archon_world_model::WorldAdvisorUnavailableReason::StoreUnavailable,
            )
        });
    if let Ok(root) = super::world_model_root() {
        let _ = archon_world_model::integration::append_surface_record(&root, &record);
    }
    record
}

pub(crate) fn record_runtime_outcome(
    config: &archon_core::config::ArchonConfig,
    record: &archon_world_model::integration::WorldAdvisorSurfaceRecord,
    actual_summary: &str,
    bundle_id: Option<&str>,
) {
    let Ok(root) = super::world_model_root() else {
        return;
    };
    let mut latent_surprise = None;
    let mut evidence_refs = bundle_id
        .map(|id| vec![format!("bundle:{id}")])
        .unwrap_or_default();
    if let Some(prediction) = &record.prediction {
        if let Ok((updated, _)) = super::predict::record_outcome_for_prediction(
            config,
            &root,
            &prediction.prediction_id,
            actual_summary,
        ) {
            latent_surprise = updated.latent_surprise;
            evidence_refs.extend(updated.evidence_refs);
        }
    }
    let outcome_id = format!(
        "{}:{}",
        record.session_id.as_deref().unwrap_or("unknown-session"),
        record.action_ref.as_deref().unwrap_or("unknown-action")
    );
    let outcome = archon_world_model::integration::WorldRuntimeOutcomeRecord {
        surface: record.surface,
        prediction_id: record
            .prediction
            .as_ref()
            .map(|prediction| prediction.prediction_id.clone()),
        session_id: record
            .session_id
            .clone()
            .unwrap_or_else(|| "unknown".into()),
        action_ref: record
            .action_ref
            .clone()
            .unwrap_or_else(|| "unknown".into()),
        actual_summary: actual_summary.to_string(),
        task_class: None,
        final_status: None,
        verification_outcomes: Vec::new(),
        user_correction_observed: false,
        plan_drift_observed: false,
        provider_incident_observed: false,
        retry_count: 0,
        latent_surprise,
        evidence_refs: evidence_refs.clone(),
        created_at: chrono::Utc::now(),
    };
    let _ = archon_world_model::integration::append_runtime_outcome(&root, &outcome);
    if let Some(bundle_id) = bundle_id {
        let attachment = archon_world_model::integration::WorldAuditedBundleAttachment {
            bundle_id: bundle_id.to_string(),
            prediction_id: outcome.prediction_id.clone(),
            outcome_id,
            evidence_refs,
            created_at: chrono::Utc::now(),
        };
        let _ = archon_world_model::integration::append_bundle_attachment(&root, &attachment);
    }
}

pub(crate) fn record_runtime_guardrail_outcome(
    config: &archon_core::config::ArchonConfig,
    record: &archon_world_model::integration::WorldAdvisorSurfaceRecord,
    outcome: &mut archon_world_model::WorldGuardrailOutcome,
    bundle_id: Option<&str>,
) {
    let Ok(root) = super::world_model_root() else {
        return;
    };
    let mut latent_surprise = None;
    let mut evidence_refs = outcome.evidence_refs.clone();
    if let Some(bundle_id) = bundle_id {
        evidence_refs.push(format!("bundle:{bundle_id}"));
    }
    if let Some(prediction) = &record.prediction {
        if let Ok((updated, _)) = super::predict::record_outcome_for_prediction(
            config,
            &root,
            &prediction.prediction_id,
            &outcome.actual_summary,
        ) {
            latent_surprise = updated.latent_surprise;
            evidence_refs.extend(updated.evidence_refs);
        }
    }
    evidence_refs.sort();
    evidence_refs.dedup();
    outcome.latent_surprise = latent_surprise;
    outcome.evidence_refs = evidence_refs.clone();

    let runtime_outcome = archon_world_model::integration::WorldRuntimeOutcomeRecord {
        surface: record.surface,
        prediction_id: record
            .prediction
            .as_ref()
            .map(|prediction| prediction.prediction_id.clone()),
        session_id: record
            .session_id
            .clone()
            .unwrap_or_else(|| "unknown".into()),
        action_ref: record
            .action_ref
            .clone()
            .unwrap_or_else(|| "unknown".into()),
        actual_summary: outcome.actual_summary.clone(),
        task_class: Some(outcome.task_class),
        final_status: Some(outcome.final_status),
        verification_outcomes: outcome.verification_outcomes.clone(),
        user_correction_observed: outcome.user_correction_observed,
        plan_drift_observed: outcome.plan_drift_observed,
        provider_incident_observed: outcome.provider_incident_observed,
        retry_count: outcome.retry_count,
        latent_surprise,
        evidence_refs: evidence_refs.clone(),
        created_at: chrono::Utc::now(),
    };
    let _ = archon_world_model::integration::append_runtime_outcome(&root, &runtime_outcome);
    if let Some(bundle_id) = bundle_id {
        let attachment = archon_world_model::integration::WorldAuditedBundleAttachment {
            bundle_id: bundle_id.to_string(),
            prediction_id: runtime_outcome.prediction_id.clone(),
            outcome_id: outcome.outcome_id.clone(),
            evidence_refs,
            created_at: chrono::Utc::now(),
        };
        let _ = archon_world_model::integration::append_bundle_attachment(&root, &attachment);
    }
}

pub(crate) fn record_runtime_counterfactual_advice(
    config: &archon_core::config::ArchonConfig,
    surface: archon_world_model::integration::WorldAdvisorSurface,
    task: &str,
    choices: &[(&str, &str)],
) -> Option<PathBuf> {
    runtime_counterfactual_advice(config, surface, task, choices).ok()
}

fn runtime_advisory_record(
    config: &archon_core::config::ArchonConfig,
    surface: archon_world_model::integration::WorldAdvisorSurface,
    session_id: &str,
    action_ref: &str,
    summary: &str,
) -> Result<archon_world_model::integration::WorldAdvisorSurfaceRecord> {
    if !config.learning.world_model.enabled {
        return Ok(
            archon_world_model::integration::WorldAdvisorSurfaceRecord::unavailable(
                surface,
                archon_world_model::WorldAdvisorUnavailableReason::StoreUnavailable,
            ),
        );
    }
    let root = super::world_model_root()?;
    let stats = super::load_world_model_stats()?;
    let active_model_id = super::active_model_id()?;
    match super::predict::persist_active_checkpoint_prediction(
        config,
        &root,
        stats,
        active_model_id.clone(),
        session_id,
        action_ref,
        summary,
    ) {
        Ok(Some((prediction, _))) => {
            return Ok(archon_world_model::integration::WorldAdvisorSurfaceRecord {
                surface,
                prediction: Some(archon_world_model::WorldPrediction {
                    prediction_id: prediction.prediction_id,
                    model_id: prediction.model_id,
                    predicted_next_state_summary: prediction.predicted_next_state_summary,
                    guardrail_scores: prediction.guardrail_scores,
                    evidence_refs: prediction.evidence_refs,
                    created_at: prediction.created_at,
                }),
                unavailable: None,
                session_id: Some(session_id.to_string()),
                action_ref: Some(action_ref.to_string()),
                action_summary: Some(summary.to_string()),
                continue_foreground_flow: true,
                created_at: chrono::Utc::now(),
            });
        }
        Ok(None) => {}
        Err(error) => {
            return Ok(
                archon_world_model::integration::WorldAdvisorSurfaceRecord::unavailable(
                    surface,
                    runtime_unavailable_reason_from_error(&error),
                )
                .with_context(session_id, action_ref, summary),
            );
        }
    }
    let advisor = archon_world_model::WorldAdvisor::new(
        archon_world_model::WorldAdvisorConfig {
            thresholds: super::cold_start_thresholds(config),
            active_model_id,
            training_in_progress: false,
        },
        stats,
    );
    let decision = advisor.evaluate(&archon_world_model::WorldAdvisorContext {
        session_id: session_id.to_string(),
        action_ref: action_ref.to_string(),
        action_summary: summary.to_string(),
    });
    Ok(
        archon_world_model::integration::WorldAdvisorSurfaceRecord::from_decision(
            surface, decision,
        )
        .with_context(session_id, action_ref, summary),
    )
}

fn runtime_unavailable_reason_from_error(
    error: &anyhow::Error,
) -> archon_world_model::WorldAdvisorUnavailableReason {
    let message = error.to_string();
    if message.contains("JepaCheckpointMissing") {
        archon_world_model::WorldAdvisorUnavailableReason::JepaCheckpointMissing
    } else if message.contains("JepaCheckpointInvalid") {
        archon_world_model::WorldAdvisorUnavailableReason::JepaCheckpointInvalid
    } else if message.contains("JepaEncoderFailed") {
        archon_world_model::WorldAdvisorUnavailableReason::JepaEncoderFailed
    } else if message.contains("JepaDimensionMismatch") {
        archon_world_model::WorldAdvisorUnavailableReason::JepaDimensionMismatch
    } else if message.contains("JepaLatencyExceeded") {
        archon_world_model::WorldAdvisorUnavailableReason::JepaLatencyExceeded
    } else if message.contains("JepaBackendProbeFailed") {
        archon_world_model::WorldAdvisorUnavailableReason::JepaBackendProbeFailed
    } else if message.contains("JepaBackendNativeStageFailed") {
        archon_world_model::WorldAdvisorUnavailableReason::JepaBackendNativeStageFailed
    } else if message.contains("JepaBackendHostFallbackRejected") {
        archon_world_model::WorldAdvisorUnavailableReason::JepaBackendHostFallbackRejected
    } else if message.contains("JepaBackendParityFailed") {
        archon_world_model::WorldAdvisorUnavailableReason::JepaBackendParityFailed
    } else if message.contains("JepaBackendHardwareValidationMissing") {
        archon_world_model::WorldAdvisorUnavailableReason::JepaBackendHardwareValidationMissing
    } else if message.contains("JepaBackendUnavailable") {
        archon_world_model::WorldAdvisorUnavailableReason::JepaBackendUnavailable
    } else {
        archon_world_model::WorldAdvisorUnavailableReason::StoreUnavailable
    }
}

pub(crate) fn record_provider_runtime_advisory(session_id: &str, action_ref: &str, summary: &str) {
    let Ok(config) = archon_core::config::load_config() else {
        return;
    };
    let _ = record_runtime_advisory(
        &config,
        archon_world_model::integration::WorldAdvisorSurface::ProviderRuntime,
        session_id,
        action_ref,
        summary,
    );
}

#[derive(Debug, Serialize)]
struct RuntimeCounterfactualRecord {
    record_id: String,
    surface: archon_world_model::integration::WorldAdvisorSurface,
    task: String,
    scores: Vec<CounterfactualScore>,
    shadow: archon_world_model::shadow::ShadowPlanReport,
    created_at: DateTime<Utc>,
}

fn runtime_counterfactual_advice(
    config: &archon_core::config::ArchonConfig,
    surface: archon_world_model::integration::WorldAdvisorSurface,
    task: &str,
    choices: &[(&str, &str)],
) -> Result<PathBuf> {
    if choices.is_empty() {
        anyhow::bail!("runtime counterfactual advice requires candidate choices");
    }
    let root = super::world_model_root()?;
    let adapter = super::embedding_runtime::build_embedding_adapter(config)?;
    let rows = WorldModelStore::open(&root)?.load_rows()?;
    let history = counterfactual_history(&rows, adapter.as_ref())?;
    if history.is_empty() {
        anyhow::bail!("runtime counterfactual advice requires historical rows");
    }
    let advisor = archon_world_model::counterfactual::KnnCounterfactualAdvisor::new(history, 3)?;
    let mut scores = Vec::new();
    for (id, summary) in choices {
        let embedding = embed_runtime_choice(adapter.as_ref(), id, task, summary)?;
        scores.push(advisor.score(id, &embedding)?);
    }
    scores.sort_by(|left, right| {
        let left_score = left.estimated_success - left.estimated_risk;
        let right_score = right.estimated_success - right.estimated_risk;
        right_score.total_cmp(&left_score)
    });
    let relevance = scores
        .iter()
        .map(|score| {
            (
                score.candidate_id.clone(),
                (score.estimated_success - score.estimated_risk).max(0.0) * 3.0,
            )
        })
        .collect::<Vec<_>>();
    let shadow = archon_world_model::shadow::rank_candidate_actions(&scores, &relevance);
    let record = RuntimeCounterfactualRecord {
        record_id: format!("world-runtime-counterfactual-{}", uuid::Uuid::new_v4()),
        surface,
        task: task.to_string(),
        scores,
        shadow,
        created_at: Utc::now(),
    };
    let dir = root.join("counterfactuals");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.json", record.record_id));
    std::fs::write(&path, serde_json::to_vec_pretty(&record)?)?;
    Ok(path)
}

fn counterfactual_history(
    rows: &[WorldTraceRow],
    adapter: &dyn WorldEmbeddingAdapter,
) -> Result<Vec<CounterfactualExample>> {
    rows.iter()
        .filter_map(|row| row.redacted_excerpt.as_deref().map(|text| (row, text)))
        .map(|(row, text)| {
            Ok(CounterfactualExample {
                action_id: row.row_id.clone(),
                action_embedding: embed_text(adapter, &row.row_id, text)?,
                observed_success: row_success(row),
                observed_risk: row_risk(row),
            })
        })
        .collect()
}

fn embed_runtime_choice(
    adapter: &dyn WorldEmbeddingAdapter,
    id: &str,
    task: &str,
    summary: &str,
) -> Result<Vec<f32>> {
    embed_text(adapter, id, &format!("task={task} choice={summary}"))
}

fn embed_text(adapter: &dyn WorldEmbeddingAdapter, id: &str, text: &str) -> Result<Vec<f32>> {
    Ok(adapter
        .embed(&EmbeddingRequest {
            text: text.to_string(),
            source_hash: id.to_string(),
            redaction_policy: "world-model-default-redacted".into(),
        })?
        .values)
}

fn row_success(row: &WorldTraceRow) -> f32 {
    match row.labels.success {
        Some(true) => 1.0,
        Some(false) => 0.0,
        None if row.labels.failure => 0.0,
        None => 0.5,
    }
}

fn row_risk(row: &WorldTraceRow) -> f32 {
    let mut risk: f32 = 0.0;
    if row.labels.failure {
        risk += 0.35;
    }
    if row.labels.retry {
        risk += 0.20;
    }
    if row.labels.provider_incident {
        risk += 0.20;
    }
    if row.labels.verification_needed || row.labels.plan_drift {
        risk += 0.15;
    }
    risk.min(1.0)
}
