use std::path::PathBuf;

use anyhow::Result;

#[path = "runtime_counterfactual.rs"]
mod runtime_counterfactual;
use runtime_counterfactual::runtime_counterfactual_advice;

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
        match super::predict::record_outcome_for_prediction(
            config,
            &root,
            &prediction.prediction_id,
            actual_summary,
        ) {
            Ok((updated, _)) => {
                latent_surprise = updated.latent_surprise;
                evidence_refs.extend(updated.evidence_refs);
            }
            Err(error) => record_prediction_outcome_failure(&error, &mut evidence_refs),
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
    record_runtime_guardrail_outcome_at_root(config, &root, record, outcome, bundle_id);
}

pub(super) fn record_runtime_guardrail_outcome_at_root(
    config: &archon_core::config::ArchonConfig,
    root: &std::path::Path,
    record: &archon_world_model::integration::WorldAdvisorSurfaceRecord,
    outcome: &mut archon_world_model::WorldGuardrailOutcome,
    bundle_id: Option<&str>,
) {
    let mut latent_surprise = None;
    let mut evidence_refs = outcome.evidence_refs.clone();
    if let Some(bundle_id) = bundle_id {
        evidence_refs.push(format!("bundle:{bundle_id}"));
    }
    if let Some(prediction) = &record.prediction {
        match super::predict::record_outcome_for_prediction(
            config,
            root,
            &prediction.prediction_id,
            &outcome.actual_summary,
        ) {
            Ok((updated, _)) => {
                latent_surprise = updated.latent_surprise;
                evidence_refs.extend(updated.evidence_refs);
            }
            Err(error) => record_prediction_outcome_failure(&error, &mut evidence_refs),
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
    let _ = archon_world_model::integration::append_runtime_outcome(root, &runtime_outcome);
    if let Some(bundle_id) = bundle_id {
        let attachment = archon_world_model::integration::WorldAuditedBundleAttachment {
            bundle_id: bundle_id.to_string(),
            prediction_id: runtime_outcome.prediction_id.clone(),
            outcome_id: outcome.outcome_id.clone(),
            evidence_refs,
            created_at: chrono::Utc::now(),
        };
        let _ = archon_world_model::integration::append_bundle_attachment(root, &attachment);
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

pub(super) fn record_runtime_guardrail_outcome_for_decision_at_root(
    config: &archon_core::config::ArchonConfig,
    root: &std::path::Path,
    decision: &archon_world_model::WorldGuardrailDecision,
    session_id: &str,
    action_ref: &str,
    action_summary: &str,
    outcome: &mut archon_world_model::WorldGuardrailOutcome,
) {
    let Some(prediction_id) = decision.prediction_id.as_deref() else {
        return;
    };
    match super::predict::load_prediction(root, prediction_id) {
        Ok(Some(prediction)) => {
            let record = surface_record_for_prediction(
                decision.surface,
                session_id,
                action_ref,
                action_summary,
                prediction,
            );
            record_runtime_guardrail_outcome_at_root(config, root, &record, outcome, None);
        }
        Ok(None) => record_prediction_outcome_failure(
            &anyhow::anyhow!("prediction not found: {prediction_id}"),
            &mut outcome.evidence_refs,
        ),
        Err(error) => record_prediction_outcome_failure(&error, &mut outcome.evidence_refs),
    }
    outcome.evidence_refs.sort();
    outcome.evidence_refs.dedup();
}

fn surface_record_for_prediction(
    surface: archon_world_model::integration::WorldAdvisorSurface,
    session_id: &str,
    action_ref: &str,
    summary: &str,
    prediction: super::predict::PersistedPrediction,
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
    archon_world_model::integration::WorldAdvisorSurfaceRecord::from_decision(surface, decision)
        .with_context(session_id, action_ref, summary)
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

fn record_prediction_outcome_failure(error: &anyhow::Error, evidence_refs: &mut Vec<String>) {
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

fn runtime_unavailable_reason_from_error(
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_stored_trace_has_typed_runtime_unavailable_reason() {
        let reason = runtime_unavailable_reason_from_error(&anyhow::anyhow!(
            "StoredTraceUnavailable: action row not found: missing-action"
        ));

        assert_eq!(
            reason,
            archon_world_model::WorldAdvisorUnavailableReason::StoredTraceUnavailable
        );
    }

    #[test]
    fn prediction_outcome_failure_adds_typed_unavailable_evidence() {
        let mut evidence_refs = Vec::new();

        record_prediction_outcome_failure(
            &anyhow::anyhow!("StoredTraceUnavailable: target window crosses session boundary"),
            &mut evidence_refs,
        );

        assert_eq!(
            evidence_refs,
            vec!["prediction_outcome_unavailable:stored_trace_unavailable"]
        );
    }
}
