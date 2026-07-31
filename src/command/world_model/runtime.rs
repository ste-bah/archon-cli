use std::path::PathBuf;

#[path = "runtime_counterfactual.rs"]
mod runtime_counterfactual;
use runtime_counterfactual::runtime_counterfactual_advice;

#[path = "runtime_advisory_record.rs"]
mod runtime_advisory_record;
#[cfg(test)]
use runtime_advisory_record::runtime_unavailable_reason_from_error;
use runtime_advisory_record::{
    record_prediction_outcome_failure, runtime_advisory_record, surface_record_for_prediction,
};

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

/// Build the world-model prediction context for the live cognitive advisory.
///
/// Returns the advisor plus the state describing which model is live. The
/// state is deliberately `shadow_only`: predictions are recorded and can be
/// compared against outcomes, but `WorldModelScorer` will not let them
/// influence any decision until that flag is cleared — which should happen
/// only once `archon world eval` reports the candidate beating the
/// nearest-neighbour baseline.
pub(crate) fn runtime_prediction_context(
    config: &archon_core::config::ArchonConfig,
) -> Option<(
    archon_world_model::WorldAdvisor,
    archon_cognitive::WorldModelState,
)> {
    if !config.learning.world_model.enabled {
        return None;
    }
    let stats = super::load_world_model_stats().ok()?;
    let active_model_id = super::active_model_id().ok().flatten();
    let advisor = archon_world_model::WorldAdvisor::new(
        archon_world_model::WorldAdvisorConfig {
            thresholds: super::cold_start_thresholds(config),
            active_model_id: active_model_id.clone(),
            training_in_progress: false,
        },
        stats,
    );
    // Archon's own behaviour is part of the data-generating process, so a model
    // trained against a different build learned a feature -> outcome
    // relationship that no longer holds. Such a model keeps returning confident
    // predictions while being quietly wrong, which is the failure this check
    // exists to prevent.
    let build_matches = active_model_trained_on_current_build(active_model_id.as_deref());
    if active_model_id.is_some() && !build_matches {
        tracing::warn!(
            active_model_id = ?active_model_id,
            running_build = %archon_world_model::build_stamp(),
            "active world model was trained against a different archon build; \
             holding it in shadow until it is retrained and re-evaluated"
        );
    }

    // Live scoring is still gated on an evaluation that has never been run, so
    // everything stays in shadow regardless. Kept as a named binding rather than
    // a literal so that lifting it is a single deliberate edit which cannot
    // accidentally bypass the build check above.
    let promoted_for_runtime = false;

    let state = archon_cognitive::WorldModelState {
        active_model_kind: active_model_id
            .as_ref()
            .map(|_| archon_cognitive::ModelKind::LatentTransition),
        active_model_id,
        jepa_promoted: false,
        shadow_only: !promoted_for_runtime || !build_matches,
    };
    Some((advisor, state))
}

/// Whether the active checkpoint was trained against the running archon build.
///
/// Read once at session wiring rather than per turn. Anything unreadable — no
/// active model, a missing or corrupt checkpoint, or a checkpoint predating
/// `trained_on_build` — is treated as "does not match", which keeps the model
/// in shadow. Failing closed is right here: the cost of a needless shadow is a
/// lost opportunity, while the cost of trusting a stale model is silently wrong
/// risk scores steering real decisions.
fn active_model_trained_on_current_build(active_model_id: Option<&str>) -> bool {
    let Some(model_id) = active_model_id else {
        return false;
    };
    let Ok(root) = super::world_model_root() else {
        return false;
    };
    let Ok(registry) = archon_world_model::registry::ModelRegistry::open(root) else {
        return false;
    };
    let Ok(record) = registry.load_cpu_candidate(model_id) else {
        return false;
    };
    record.model.metadata.trained_on_build.as_deref()
        == Some(archon_world_model::build_stamp().as_str())
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

    /// No active model means nothing to trust, so the guard must not report a
    /// match — otherwise a fresh install would start out looking validated.
    #[test]
    fn build_guard_reports_no_match_without_an_active_model() {
        assert!(!super::active_model_trained_on_current_build(None));
    }

    /// An unreadable or absent checkpoint must fail closed. The cost of a
    /// needless shadow is a lost opportunity; the cost of trusting a stale
    /// model is silently wrong risk scores steering real decisions.
    #[test]
    fn build_guard_fails_closed_when_the_checkpoint_cannot_be_read() {
        assert!(!super::active_model_trained_on_current_build(Some(
            "world-model-candidate-does-not-exist"
        )));
    }

    /// The stamp must carry the commit, not just the release version: six
    /// commits can share a version, and a corpus spanning them would not be
    /// segmentable by version alone.
    #[test]
    fn build_stamp_identifies_the_commit_not_just_the_release() {
        let stamp = archon_world_model::build_stamp();
        let (version, commit) = stamp.split_once('+').expect("stamp is <version>+<commit>");
        assert!(!version.is_empty());
        assert!(!commit.is_empty());
    }
}
