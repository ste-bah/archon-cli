use std::path::Path;
use std::time::Instant;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use archon_world_model::embedding::{EmbeddingRequest, WorldEmbeddingAdapter};
use archon_world_model::guardrail::GuardrailRiskScores;
use archon_world_model::jepa::JEPA_MODEL_KIND;
use archon_world_model::registry::{LATENT_TRANSITION_MODEL_KIND, ModelRegistry};
use archon_world_model::representation::{
    TraceAction, TraceWindowBuilder, WorldRepresentationAdapter,
};
use archon_world_model::schema::{WorldActionKind, WorldTraceRow};

use super::embedding_runtime::build_embedding_adapter;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct PersistedPrediction {
    pub prediction_id: String,
    pub model_id: String,
    #[serde(default = "default_prediction_model_kind")]
    pub model_kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub representation_source: Option<String>,
    pub session_id: String,
    pub action_ref: String,
    pub action_summary: String,
    pub predicted_next_state_summary: String,
    #[serde(default)]
    pub predicted_next_state: Vec<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actual_next_state_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actual_next_state: Option<Vec<f32>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latent_surprise: Option<f32>,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guardrail_scores: Option<GuardrailRiskScores>,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome_recorded_at: Option<DateTime<Utc>>,
}

struct PredictionInference {
    summary: String,
    vector: Vec<f32>,
    model_kind: String,
    representation_source: String,
    guardrail_scores: Option<GuardrailRiskScores>,
}

fn default_prediction_model_kind() -> String {
    LATENT_TRANSITION_MODEL_KIND.into()
}

pub(super) fn render_active_checkpoint_prediction(
    config: &archon_core::config::ArchonConfig,
    root: &Path,
    stats: archon_world_model::ColdStartStats,
    active_model_id: Option<String>,
    session_id: &str,
    action_ref: &str,
    summary: &str,
) -> Option<String> {
    match persist_active_checkpoint_prediction(
        config,
        root,
        stats,
        active_model_id,
        session_id,
        action_ref,
        summary,
    ) {
        Ok(Some((prediction, prediction_path))) => Some(format!(
            "World Model Prediction\n\
             ======================\n\
             Prediction id: {}\n\
             Session: {session_id}\n\
             Action ref: {action_ref}\n\
             Model: {}\n\
             Model kind: {}\n\
             Representation: {}\n\
             Inference: active_checkpoint\n\
             Prediction: {}\n\
             Prediction record: {}",
            prediction.prediction_id,
            prediction.model_id,
            prediction.model_kind,
            prediction
                .representation_source
                .as_deref()
                .unwrap_or("generic_embedding"),
            prediction.predicted_next_state_summary,
            prediction_path.display()
        )),
        Ok(None) => None,
        Err(error) => Some(super::render_unavailable_prediction(
            session_id,
            action_ref,
            unavailable_reason_from_error(&error),
        )),
    }
}

pub(super) fn persist_active_checkpoint_prediction(
    config: &archon_core::config::ArchonConfig,
    root: &Path,
    stats: archon_world_model::ColdStartStats,
    active_model_id: Option<String>,
    session_id: &str,
    action_ref: &str,
    summary: &str,
) -> anyhow::Result<Option<(PersistedPrediction, std::path::PathBuf)>> {
    let Some(model_id) = active_model_id else {
        return Ok(None);
    };
    let advisor = archon_world_model::WorldAdvisor::new(
        archon_world_model::WorldAdvisorConfig {
            thresholds: super::cold_start_thresholds(config),
            active_model_id: Some(model_id.clone()),
            training_in_progress: false,
        },
        stats,
    );
    let context = archon_world_model::WorldAdvisorContext {
        session_id: session_id.to_string(),
        action_ref: action_ref.to_string(),
        action_summary: summary.to_string(),
    };
    if advisor.evaluate(&context).prediction.is_none() {
        return Ok(None);
    }

    let inference =
        predict_with_checkpoint(config, root, &model_id, session_id, action_ref, summary)?;
    let prediction = PersistedPrediction {
        prediction_id: format!("world-prediction-{}", uuid::Uuid::new_v4()),
        model_id,
        model_kind: inference.model_kind,
        representation_source: Some(inference.representation_source),
        session_id: session_id.to_string(),
        action_ref: action_ref.to_string(),
        action_summary: summary.to_string(),
        predicted_next_state_summary: inference.summary,
        predicted_next_state: inference.vector,
        actual_next_state_summary: None,
        actual_next_state: None,
        latent_surprise: None,
        evidence_refs: vec![format!("runtime_action:{action_ref}")],
        guardrail_scores: inference.guardrail_scores,
        created_at: Utc::now(),
        outcome_recorded_at: None,
    };
    let prediction_path = write_prediction(root, &prediction)?;
    Ok(Some((prediction, prediction_path)))
}

pub(super) fn render_record_outcome(
    config: &archon_core::config::ArchonConfig,
    root: &Path,
    prediction_id: &str,
    actual_summary: &str,
) -> anyhow::Result<String> {
    let (prediction, path) =
        record_outcome_for_prediction(config, root, prediction_id, actual_summary)?;
    let surprise = prediction.latent_surprise.unwrap_or(1.0);

    Ok(format!(
        "World Model Outcome\n\
         ===================\n\
         Prediction: {prediction_id}\n\
         Actual outcome: {actual_summary}\n\
         Latent surprise: {surprise:.4}\n\
         Prediction record: {}",
        path.display()
    ))
}

pub(super) fn record_outcome_for_prediction(
    config: &archon_core::config::ArchonConfig,
    root: &Path,
    prediction_id: &str,
    actual_summary: &str,
) -> anyhow::Result<(PersistedPrediction, std::path::PathBuf)> {
    let mut prediction = load_prediction(root, prediction_id)?
        .ok_or_else(|| anyhow::anyhow!("prediction not found: {prediction_id}"))?;
    let actual_next_state = if prediction.model_kind == JEPA_MODEL_KIND {
        encode_jepa_actual_outcome(config, root, &prediction, actual_summary)?
    } else {
        let adapter = build_embedding_adapter(config)?;
        embed(
            adapter.as_ref(),
            &format!("{prediction_id}-actual"),
            actual_summary,
        )?
    };
    if prediction.predicted_next_state.len() != actual_next_state.len() {
        anyhow::bail!(
            "prediction vector dimension mismatch: expected {}, got {}",
            prediction.predicted_next_state.len(),
            actual_next_state.len()
        );
    }

    let surprise = cosine_error(&prediction.predicted_next_state, &actual_next_state);
    prediction.actual_next_state_summary = Some(actual_summary.to_string());
    prediction.actual_next_state = Some(actual_next_state);
    prediction.latent_surprise = Some(surprise);
    prediction.outcome_recorded_at = Some(Utc::now());
    let path = write_prediction(root, &prediction)?;
    Ok((prediction, path))
}

pub(super) fn load_prediction(
    root: &Path,
    prediction_id: &str,
) -> anyhow::Result<Option<PersistedPrediction>> {
    let path = prediction_path(root, prediction_id);
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(path)?;
    serde_json::from_str(&content).map(Some).map_err(Into::into)
}

fn write_prediction(
    root: &Path,
    prediction: &PersistedPrediction,
) -> anyhow::Result<std::path::PathBuf> {
    std::fs::create_dir_all(root.join("predictions"))?;
    let path = prediction_path(root, &prediction.prediction_id);
    std::fs::write(&path, serde_json::to_vec_pretty(prediction)?)?;
    Ok(path)
}

fn prediction_path(root: &Path, prediction_id: &str) -> std::path::PathBuf {
    root.join("predictions")
        .join(format!("{prediction_id}.json"))
}

fn predict_with_checkpoint(
    config: &archon_core::config::ArchonConfig,
    root: &Path,
    model_id: &str,
    session_id: &str,
    action_ref: &str,
    summary: &str,
) -> anyhow::Result<PredictionInference> {
    let registry = ModelRegistry::open(root)?;
    let active_kind = registry
        .active_model_kind()?
        .unwrap_or_else(|| LATENT_TRANSITION_MODEL_KIND.into());
    if config.learning.world_model.model_kind == JEPA_MODEL_KIND {
        if active_kind != JEPA_MODEL_KIND {
            anyhow::bail!("JepaCheckpointMissing: active model is not a JEPA checkpoint");
        }
        return predict_with_jepa_checkpoint(
            config, &registry, model_id, session_id, action_ref, summary,
        );
    }

    let candidate = registry.load_cpu_candidate(model_id)?;
    let adapter = build_embedding_adapter(config)?;
    let state = embed(adapter.as_ref(), model_id, summary)?;
    let action = embed(adapter.as_ref(), model_id, &format!("action={summary}"))?;
    let next = archon_world_model::backend::predict_next_with_backend(
        &candidate.model,
        &state,
        &action,
        candidate.model.metadata.backend,
    )?;
    let guardrail_scores = Some(guardrail_scores_from_auxiliary(
        candidate
            .model
            .predict_auxiliary(&state, &action)?
            .iter()
            .map(|prediction| (prediction.label.as_str(), prediction.probability)),
    ));
    Ok(PredictionInference {
        summary: format!(
            "next-state dim={} norm={:.4}",
            next.len(),
            vector_norm(&next)
        ),
        vector: next,
        model_kind: LATENT_TRANSITION_MODEL_KIND.into(),
        representation_source: format!("{}:{}", adapter.provider_name(), adapter.model_name()),
        guardrail_scores,
    })
}

fn predict_with_jepa_checkpoint(
    config: &archon_core::config::ArchonConfig,
    registry: &ModelRegistry,
    model_id: &str,
    session_id: &str,
    action_ref: &str,
    summary: &str,
) -> anyhow::Result<PredictionInference> {
    let started = Instant::now();
    let candidate = registry
        .load_jepa_candidate(model_id)
        .map_err(|error| anyhow::anyhow!("JepaCheckpointMissing: {error}"))?;
    candidate
        .model
        .validate_finite()
        .map_err(|error| anyhow::anyhow!("JepaCheckpointInvalid: {error}"))?;
    if candidate.model.metadata.latent_dim != config.learning.world_model.state_dim {
        anyhow::bail!(
            "JepaDimensionMismatch: expected {}, got {}",
            config.learning.world_model.state_dim,
            candidate.model.metadata.latent_dim
        );
    }
    let transition = candidate
        .model
        .transition_model
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("JepaCheckpointMissing: transition model missing"))?;
    let (window, action) = synthetic_runtime_window(session_id, action_ref, summary)?;
    let state = candidate
        .model
        .encode_state(&window)
        .map_err(|error| anyhow::anyhow!("JepaEncoderFailed: {error}"))?;
    let action = candidate
        .model
        .encode_action(&action)
        .map_err(|error| anyhow::anyhow!("JepaEncoderFailed: {error}"))?;
    let next = archon_world_model::backend::predict_next_with_backend(
        transition,
        &state,
        &action,
        transition.metadata.backend,
    )?;
    let guardrail_scores = Some(guardrail_scores_from_auxiliary(
        candidate
            .model
            .predict_auxiliary(&state, &action)?
            .iter()
            .map(|(label, probability)| (label.as_str(), *probability)),
    ));
    let elapsed_ms = started.elapsed().as_millis() as u64;
    if elapsed_ms > config.learning.world_model.jepa.max_prediction_latency_ms {
        anyhow::bail!(
            "JepaLatencyExceeded: prediction took {elapsed_ms}ms, cap={}ms",
            config.learning.world_model.jepa.max_prediction_latency_ms
        );
    }
    Ok(PredictionInference {
        summary: format!(
            "next-state dim={} norm={:.4}",
            next.len(),
            vector_norm(&next)
        ),
        vector: next,
        model_kind: JEPA_MODEL_KIND.into(),
        representation_source: format!("archon-jepa:{}", candidate.model.metadata.model_id),
        guardrail_scores,
    })
}

fn encode_jepa_actual_outcome(
    config: &archon_core::config::ArchonConfig,
    root: &Path,
    prediction: &PersistedPrediction,
    actual_summary: &str,
) -> anyhow::Result<Vec<f32>> {
    let registry = ModelRegistry::open(root)?;
    let candidate = registry
        .load_jepa_candidate(&prediction.model_id)
        .map_err(|error| anyhow::anyhow!("JepaCheckpointMissing: {error}"))?;
    if candidate.model.metadata.latent_dim != config.learning.world_model.state_dim {
        anyhow::bail!(
            "JepaDimensionMismatch: expected {}, got {}",
            config.learning.world_model.state_dim,
            candidate.model.metadata.latent_dim
        );
    }
    let (window, _) = synthetic_runtime_window(
        &prediction.session_id,
        &format!("{}-actual", prediction.action_ref),
        actual_summary,
    )?;
    candidate
        .model
        .encode_target(&window)
        .map_err(|error| anyhow::anyhow!("JepaEncoderFailed: {error}"))
}

fn synthetic_runtime_window(
    session_id: &str,
    action_ref: &str,
    summary: &str,
) -> anyhow::Result<(archon_world_model::TraceWindow, TraceAction)> {
    let mut row =
        WorldTraceRow::new(session_id, WorldActionKind::AgentAttempt).with_row_id(action_ref);
    row.redacted_excerpt = Some(summary.to_string());
    row.created_at = Utc::now();
    let action = TraceAction::from_row(&row);
    let builder = TraceWindowBuilder::new(&[row]);
    let window = builder
        .context_window(action_ref, 1)
        .map_err(|error| anyhow::anyhow!("JepaEncoderFailed: {error}"))?;
    Ok((window, action))
}

fn unavailable_reason_from_error(error: &anyhow::Error) -> &'static str {
    let message = error.to_string();
    for reason in [
        "JepaCheckpointMissing",
        "JepaCheckpointInvalid",
        "JepaEncoderFailed",
        "JepaDimensionMismatch",
        "JepaLatencyExceeded",
    ] {
        if message.contains(reason) {
            return reason;
        }
    }
    "StoreUnavailable"
}

fn embed(
    adapter: &dyn WorldEmbeddingAdapter,
    source_hash: &str,
    text: &str,
) -> anyhow::Result<Vec<f32>> {
    Ok(adapter
        .embed(&EmbeddingRequest {
            text: text.to_string(),
            source_hash: source_hash.to_string(),
            redaction_policy: "world-model-default-redacted".into(),
        })?
        .values)
}

fn vector_norm(values: &[f32]) -> f32 {
    values.iter().map(|value| value * value).sum::<f32>().sqrt()
}

fn guardrail_scores_from_auxiliary<'a>(
    predictions: impl IntoIterator<Item = (&'a str, f32)>,
) -> GuardrailRiskScores {
    let mut scores = GuardrailRiskScores::default();
    for (label, probability) in predictions {
        let probability = finite_probability(probability);
        match label {
            "failure" => scores.predicted_failure = probability,
            "retry" => scores.predicted_retry = probability,
            "provider_incident" => scores.predicted_provider_incident = probability,
            "verification_needed" => scores.predicted_verification_needed = probability,
            "user_correction" => scores.predicted_user_correction = probability,
            "plan_drift" => scores.predicted_plan_drift = probability,
            "high_cost" => scores.predicted_high_cost = probability,
            "slow_run" => scores.predicted_slow_run = probability,
            _ => {}
        }
    }
    scores
}

fn finite_probability(probability: f32) -> Option<f32> {
    probability.is_finite().then(|| probability.clamp(0.0, 1.0))
}

fn cosine_error(left: &[f32], right: &[f32]) -> f32 {
    let dot = left.iter().zip(right).map(|(a, b)| a * b).sum::<f32>();
    let left_norm = vector_norm(left);
    let right_norm = vector_norm(right);
    if left_norm == 0.0 || right_norm == 0.0 {
        1.0
    } else {
        1.0 - (dot / (left_norm * right_norm)).clamp(-1.0, 1.0)
    }
}
