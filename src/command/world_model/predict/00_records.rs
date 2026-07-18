use std::path::Path;
use std::time::Instant;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use archon_world_model::embedding::{EmbeddingRequest, WorldEmbeddingAdapter};
use archon_world_model::guardrail::GuardrailRiskScores;
use archon_world_model::integration::WorldAdvisorSurface;
use archon_world_model::jepa::JEPA_MODEL_KIND;
use archon_world_model::registry::{LATENT_TRANSITION_MODEL_KIND, ModelRegistry};
use archon_world_model::representation::{
    TraceAction, TraceWindow, TraceWindowBuilder, WorldRepresentationAdapter,
};
use archon_world_model::schema::{WorldActionKind, WorldTraceRow, WorldTraceSource};
use archon_world_model::storage::WorldModelStore;

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jepa_runtime_backend_report: Option<archon_world_model::JepaRuntimeBackendReport>,
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
    jepa_runtime_backend_report: Option<archon_world_model::JepaRuntimeBackendReport>,
}

struct PredictionTrace {
    context: TraceWindow,
    action: TraceAction,
    action_summary: String,
    evidence_refs: Vec<String>,
}

fn default_prediction_model_kind() -> String {
    LATENT_TRANSITION_MODEL_KIND.into()
}

fn runtime_action_row(
    surface: WorldAdvisorSurface,
    session_id: &str,
    action_ref: &str,
    action_summary: &str,
) -> WorldTraceRow {
    let action_kind = match surface {
        WorldAdvisorSurface::MemorySurfacing => WorldActionKind::MemorySurface,
        WorldAdvisorSurface::ToolRun => WorldActionKind::ToolCall,
        WorldAdvisorSurface::VerificationRun => WorldActionKind::Verification,
        WorldAdvisorSurface::Pipeline | WorldAdvisorSurface::PipelineStep => {
            WorldActionKind::PlanUpdate
        }
        _ => WorldActionKind::AgentAttempt,
    };
    let source = match surface {
        WorldAdvisorSurface::ProviderRuntime => WorldTraceSource::ProviderRuntime,
        WorldAdvisorSurface::MemorySurfacing => WorldTraceSource::Memory,
        WorldAdvisorSurface::AgentEvolution => WorldTraceSource::AgentEvolution,
        WorldAdvisorSurface::InteractiveSession => WorldTraceSource::Conversation,
        WorldAdvisorSurface::Pipeline | WorldAdvisorSurface::PipelineStep => {
            WorldTraceSource::PipelineBundle
        }
        _ => WorldTraceSource::ActivityEvent,
    };
    let mut action = WorldTraceRow::new(session_id, action_kind)
        .with_row_id(format!("world-runtime-action-{}", uuid::Uuid::new_v4()));
    action.source = source;
    action.redacted_excerpt = Some(format!("operation={action_ref} input={action_summary}"));
    action
}

fn prediction_trace(
    root: &Path,
    surface: WorldAdvisorSurface,
    session_id: &str,
    action_ref: &str,
    action_summary: &str,
    context_limit: usize,
) -> anyhow::Result<PredictionTrace> {
    let store = WorldModelStore::open(root)?;
    let mut rows = store.load_rows()?;
    let action_row = match rows.iter().find(|row| row.row_id == action_ref) {
        Some(row) => row.clone(),
        None => {
            let row = runtime_action_row(surface, session_id, action_ref, action_summary);
            store.persist_rows(&[row.clone()])?;
            rows.push(row.clone());
            row
        }
    };
    if action_row.session_id != session_id {
        anyhow::bail!(
            "stored action row session mismatch: expected {session_id}, got {}",
            action_row.session_id
        );
    }
    let context_rows = rows
        .iter()
        .filter(|row| row.session_id == session_id)
        .cloned()
        .collect::<Vec<_>>();
    let builder = TraceWindowBuilder::new(&context_rows);
    let context = builder
        .prior_context_window(&action_row.row_id, context_limit)
        .map_err(|error| anyhow::anyhow!("stored trace context unavailable: {error}"))?;
    Ok(PredictionTrace {
        action: TraceAction::from_row(&action_row),
        action_summary: action_row.redacted_excerpt.clone().unwrap_or_default(),
        evidence_refs: context
            .rows
            .iter()
            .map(|row| format!("world_row:{}", row.row_id))
            .collect(),
        context,
    })
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
        WorldAdvisorSurface::InteractiveSession,
        session_id,
        action_ref,
        summary,
    ) {
        Ok(Some((prediction, prediction_path))) => {
            Some(render_prediction(&prediction, &prediction_path))
        }
        Ok(None) => None,
        Err(error) => Some(super::render_unavailable_prediction(
            session_id,
            action_ref,
            unavailable_reason_from_error(&error),
        )),
    }
}

fn render_prediction(prediction: &PersistedPrediction, path: &Path) -> String {
    format!(
        "World Model Prediction\n\
         ======================\n\
         Prediction id: {}\n\
         Session: {}\n\
         Action ref: {}\n\
         Model: {}\n\
         Model kind: {}\n\
         Representation: {}\n\
         Inference: active_checkpoint\n\
         Prediction: {}\n\
         {}\
         Prediction record: {}",
        prediction.prediction_id,
        prediction.session_id,
        prediction.action_ref,
        prediction.model_id,
        prediction.model_kind,
        prediction
            .representation_source
            .as_deref()
            .unwrap_or("generic_embedding"),
        prediction.predicted_next_state_summary,
        jepa_runtime_backend_report_lines(prediction),
        path.display()
    )
}

fn prediction_is_ready(
    config: &archon_core::config::ArchonConfig,
    stats: archon_world_model::ColdStartStats,
    model_id: &str,
    session_id: &str,
    action_ref: &str,
    summary: &str,
) -> bool {
    let advisor = archon_world_model::WorldAdvisor::new(
        archon_world_model::WorldAdvisorConfig {
            thresholds: super::cold_start_thresholds(config),
            active_model_id: Some(model_id.to_string()),
            training_in_progress: false,
        },
        stats,
    );
    let context = archon_world_model::WorldAdvisorContext {
        session_id: session_id.to_string(),
        action_ref: action_ref.to_string(),
        action_summary: summary.to_string(),
    };
    advisor.evaluate(&context).prediction.is_some()
}

pub(super) fn persist_active_checkpoint_prediction(
    config: &archon_core::config::ArchonConfig,
    root: &Path,
    stats: archon_world_model::ColdStartStats,
    active_model_id: Option<String>,
    surface: WorldAdvisorSurface,
    session_id: &str,
    action_ref: &str,
    summary: &str,
) -> anyhow::Result<Option<(PersistedPrediction, std::path::PathBuf)>> {
    let Some(model_id) = active_model_id else {
        return Ok(None);
    };
    if !prediction_is_ready(config, stats, &model_id, session_id, action_ref, summary) {
        return Ok(None);
    }

    let trace = prediction_trace(
        root,
        surface,
        session_id,
        action_ref,
        summary,
        config.learning.world_model.jepa.context_window_rows,
    )?;
    let inference = predict_with_checkpoint(config, root, &model_id, &trace)?;
    let prediction = PersistedPrediction {
        prediction_id: format!("world-prediction-{}", uuid::Uuid::new_v4()),
        model_id,
        model_kind: inference.model_kind,
        representation_source: Some(inference.representation_source),
        session_id: session_id.to_string(),
        action_ref: action_ref.to_string(),
        action_summary: trace.action_summary,
        predicted_next_state_summary: inference.summary,
        predicted_next_state: inference.vector,
        actual_next_state_summary: None,
        actual_next_state: None,
        latent_surprise: None,
        evidence_refs: trace.evidence_refs,
        guardrail_scores: inference.guardrail_scores,
        jepa_runtime_backend_report: inference.jepa_runtime_backend_report,
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
