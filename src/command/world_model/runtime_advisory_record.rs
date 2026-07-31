//! Advisory-record construction for the world-model runtime hooks.
//!
//! Split out of `runtime.rs` to hold the 500-line ceiling (ERR-TUI-004).

use anyhow::Result;

pub(super) fn surface_record_for_prediction(
    surface: archon_world_model::integration::WorldAdvisorSurface,
    session_id: &str,
    action_ref: &str,
    summary: &str,
    prediction: crate::command::world_model::predict::PersistedPrediction,
) -> archon_world_model::integration::WorldAdvisorSurfaceRecord {
    archon_world_model::integration::WorldAdvisorSurfaceRecord {
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
    }
}

fn fallback_advisory_record(
    config: &archon_core::config::ArchonConfig,
    surface: archon_world_model::integration::WorldAdvisorSurface,
    stats: archon_world_model::ColdStartStats,
    active_model_id: Option<String>,
    session_id: &str,
    action_ref: &str,
    summary: &str,
) -> archon_world_model::integration::WorldAdvisorSurfaceRecord {
    let advisor = archon_world_model::WorldAdvisor::new(
        archon_world_model::WorldAdvisorConfig {
            thresholds: crate::command::world_model::cold_start_thresholds(config),
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
    archon_world_model::integration::WorldAdvisorSurfaceRecord::from_decision(surface, decision)
        .with_context(session_id, action_ref, summary)
}

pub(super) fn runtime_advisory_record(
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
    let root = crate::command::world_model::world_model_root()?;
    let stats = crate::command::world_model::load_world_model_stats()?;
    let active_model_id = crate::command::world_model::active_model_id()?;
    match crate::command::world_model::predict::persist_active_checkpoint_prediction(
        config,
        &root,
        stats,
        active_model_id.clone(),
        session_id,
        action_ref,
        summary,
    ) {
        Ok(Some((prediction, _))) => {
            return Ok(surface_record_for_prediction(
                surface, session_id, action_ref, summary, prediction,
            ));
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
    Ok(fallback_advisory_record(
        config,
        surface,
        stats,
        active_model_id,
        session_id,
        action_ref,
        summary,
    ))
}

pub(super) fn record_prediction_outcome_failure(
    error: &anyhow::Error,
    evidence_refs: &mut Vec<String>,
) {
    let reason = unavailable_reason_code(runtime_unavailable_reason_from_error(error));
    evidence_refs.push(format!("prediction_outcome_unavailable:{reason}"));
    tracing::warn!(%error, %reason, "world-model prediction outcome unavailable");
}

fn unavailable_reason_code(reason: archon_world_model::WorldAdvisorUnavailableReason) -> String {
    serde_json::to_value(reason)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| "store_unavailable".into())
}

pub(super) fn runtime_unavailable_reason_from_error(
    error: &anyhow::Error,
) -> archon_world_model::WorldAdvisorUnavailableReason {
    let message = error.to_string();
    if message.contains("StoredTraceUnavailable") {
        archon_world_model::WorldAdvisorUnavailableReason::StoredTraceUnavailable
    } else if message.contains("JepaCheckpointMissing") {
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
