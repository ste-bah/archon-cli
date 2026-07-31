use std::path::Path;

use anyhow::{Result, bail};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde::Serialize;

use archon_world_model::counterfactual::{CounterfactualExample, KnnCounterfactualAdvisor};
use archon_world_model::embedding::{EmbeddingRequest, WorldEmbeddingAdapter};
use archon_world_model::schema::WorldTraceRow;
use archon_world_model::storage::WorldModelStore;

use super::embedding_runtime::build_embedding_adapter;

#[derive(Debug, Clone, Deserialize)]
struct ActionFile {
    actions: Vec<ActionInput>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum ActionFileFormat {
    Wrapped(ActionFile),
    Array(Vec<ActionInput>),
}

#[derive(Debug, Clone, Deserialize)]
struct ActionInput {
    id: Option<String>,
    summary: Option<String>,
    action: Option<String>,
    text: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct PersistedCounterfactualRecord {
    record_id: String,
    task: String,
    scores: Vec<archon_world_model::counterfactual::CounterfactualScore>,
    evidence_refs: Vec<String>,
    created_at: DateTime<Utc>,
}

pub(super) fn render_score_actions(
    config: &archon_core::config::ArchonConfig,
    root: &Path,
    task: &str,
    actions_path: &Path,
) -> Result<String> {
    let adapter = build_embedding_adapter(config)?;
    let rows = WorldModelStore::open(root)?.load_rows()?;
    let history = counterfactual_examples(&rows, adapter.as_ref())?;
    if history.is_empty() {
        bail!("score-actions requires historical world-model rows");
    }
    let actions = load_actions(actions_path)?;
    if actions.is_empty() {
        bail!("score-actions requires at least one candidate action");
    }

    let advisor = KnnCounterfactualAdvisor::new(history, 3)?;
    let mut scored = Vec::new();
    for (idx, action) in actions.iter().enumerate() {
        let id = action.id.clone().unwrap_or_else(|| format!("action-{idx}"));
        let text = action_text(task, action);
        let embedding = embed_text(adapter.as_ref(), &id, &text)?;
        scored.push(advisor.score(&id, &embedding)?);
    }
    // Phase 4: let the learned dynamics model adjust the k-NN ranking.
    //
    // k-NN answers "what happened after actions that looked like this one".
    // The transition model answers "what does this action lead to", which is
    // the question worth asking when choosing between candidates rather than
    // blocking one. Ranking uses both.
    let model_penalties = model_risk_penalties(root, &scored, &actions, task, adapter.as_ref());
    scored.sort_by(|left, right| {
        // Subtractive, exactly as in the cognitive scorer: an unvalidated model
        // may lower a candidate but must never promote one on the strength of
        // its own prediction.
        let score_of = |s: &archon_world_model::counterfactual::CounterfactualScore| {
            s.estimated_success
                - s.estimated_risk
                - model_penalties.get(&s.candidate_id).copied().unwrap_or(0.0)
        };
        score_of(right).total_cmp(&score_of(left))
    });

    let mut output = format!(
        "World Model Action Scores\n\
         =========================\n\
         Task: {task}\n\
         Historical rows: {}\n\
         Candidate actions: {}\n",
        rows.len(),
        scored.len()
    );
    for (rank, score) in scored.iter().enumerate() {
        output.push_str(&format!(
            "{}. {} success={:.3} risk={:.3} neighbors={}\n",
            rank + 1,
            score.candidate_id,
            score.estimated_success,
            score.estimated_risk,
            score.neighbors.len()
        ));
    }
    let record_path = write_counterfactual_record(root, task, &scored)?;
    output.push_str(&format!(
        "Calibration: similarity-based, not causal\nScore record: {}",
        record_path.display()
    ));
    Ok(output)
}

pub(super) fn render_explain(root: &Path, prediction_id: &str) -> String {
    match super::predict::load_prediction(root, prediction_id) {
        Ok(Some(prediction)) => {
            let mut output = format!(
                "World Model Explain\n\
                 ===================\n\
                 Prediction: {}\n\
                 Model: {}\n\
                 Session: {}\n\
                 Action ref: {}\n\
                 Summary: {}\n\
                 Predicted next state: {}",
                prediction.prediction_id,
                prediction.model_id,
                prediction.session_id,
                prediction.action_ref,
                prediction.action_summary,
                prediction.predicted_next_state_summary
            );
            if let Some(actual) = prediction.actual_next_state_summary.as_deref() {
                output.push_str(&format!(
                    "\nActual outcome: {actual}\nLatent surprise: {:.4}",
                    prediction.latent_surprise.unwrap_or(0.0)
                ));
            } else {
                output.push_str("\nOutcome: pending");
            }
            if !prediction.evidence_refs.is_empty() {
                output.push_str(&format!(
                    "\nEvidence refs: {}",
                    prediction.evidence_refs.join(", ")
                ));
            }
            output
        }
        _ => format!(
            "World Model Explain\n\
             ===================\n\
             Prediction: {prediction_id}\n\
             Status: not_found"
        ),
    }
}

fn write_counterfactual_record(
    root: &Path,
    task: &str,
    scores: &[archon_world_model::counterfactual::CounterfactualScore],
) -> Result<std::path::PathBuf> {
    let record = PersistedCounterfactualRecord {
        record_id: format!("world-counterfactual-{}", uuid::Uuid::new_v4()),
        task: task.to_string(),
        scores: scores.to_vec(),
        evidence_refs: scores
            .iter()
            .flat_map(|score| {
                score
                    .neighbors
                    .iter()
                    .map(|neighbor| format!("world_row:{}", neighbor.action_id))
            })
            .collect(),
        created_at: Utc::now(),
    };
    let dir = root.join("counterfactuals");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.json", record.record_id));
    std::fs::write(&path, serde_json::to_vec_pretty(&record)?)?;
    Ok(path)
}

fn load_actions(path: &Path) -> Result<Vec<ActionInput>> {
    let content = std::fs::read_to_string(path)?;
    match serde_json::from_str::<ActionFileFormat>(&content)? {
        ActionFileFormat::Wrapped(file) => Ok(file.actions),
        ActionFileFormat::Array(actions) => Ok(actions),
    }
}

fn counterfactual_examples(
    rows: &[WorldTraceRow],
    adapter: &dyn WorldEmbeddingAdapter,
) -> Result<Vec<CounterfactualExample>> {
    rows.iter()
        .filter(|row| row.redacted_excerpt.is_some())
        .map(|row| {
            Ok(CounterfactualExample {
                action_id: row.row_id.clone(),
                action_embedding: embed_text(
                    adapter,
                    &row.row_id,
                    row.redacted_excerpt.as_deref().unwrap_or_default(),
                )?,
                observed_success: success_score(row),
                observed_risk: risk_score(row),
            })
        })
        .collect()
}

fn embed_text(adapter: &dyn WorldEmbeddingAdapter, id: &str, text: &str) -> Result<Vec<f32>> {
    let vector = adapter.embed(&EmbeddingRequest {
        text: text.to_string(),
        source_hash: id.to_string(),
        redaction_policy: "world-model-default-redacted".into(),
    })?;
    Ok(vector.values)
}

fn action_text(task: &str, action: &ActionInput) -> String {
    format!(
        "task={} action={}",
        task,
        action
            .summary
            .as_deref()
            .or(action.action.as_deref())
            .or(action.text.as_deref())
            .unwrap_or_default()
    )
}

fn success_score(row: &WorldTraceRow) -> f32 {
    match row.labels.success {
        Some(true) => 1.0,
        Some(false) => 0.0,
        None => {
            if row.labels.failure {
                0.0
            } else {
                0.5
            }
        }
    }
}

fn risk_score(row: &WorldTraceRow) -> f32 {
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

/// Per-candidate risk penalty from the active transition model.
///
/// Returns an empty map when there is no active model, the checkpoint cannot be
/// read, or an embedding fails — ranking then falls back to the unmodified
/// k-NN ordering. Fail-open is deliberate: the model is unvalidated, so its
/// absence must cost nothing and its presence must not be required.
fn model_risk_penalties(
    root: &Path,
    scored: &[archon_world_model::counterfactual::CounterfactualScore],
    actions: &[ActionInput],
    task: &str,
    adapter: &dyn archon_world_model::embedding::WorldEmbeddingAdapter,
) -> std::collections::BTreeMap<String, f32> {
    let mut penalties = std::collections::BTreeMap::new();
    let Some(model) = active_transition_model(root) else {
        return penalties;
    };
    for (idx, action) in actions.iter().enumerate() {
        let id = action.id.clone().unwrap_or_else(|| format!("action-{idx}"));
        if !scored.iter().any(|score| score.candidate_id == id) {
            continue;
        }
        let Ok(embedding) = embed_text(adapter, &id, &action_text(task, action)) else {
            continue;
        };
        if embedding.len() != model.metadata.state_dim {
            continue;
        }
        let Ok(predictions) = model.predict_auxiliary(&embedding, &embedding) else {
            continue;
        };
        // Mean of the non-success heads: the risk the model attributes to
        // taking this action. Success is excluded because it is the term the
        // k-NN estimate already supplies.
        let risk_heads: Vec<f32> = predictions
            .iter()
            .filter(|prediction| prediction.label != "success")
            .map(|prediction| prediction.probability)
            .collect();
        if risk_heads.is_empty() {
            continue;
        }
        let penalty = risk_heads.iter().sum::<f32>() / risk_heads.len() as f32;
        penalties.insert(id, penalty.clamp(0.0, 1.0));
    }
    penalties
}

/// Load the promoted transition model, if there is one.
fn active_transition_model(
    root: &Path,
) -> Option<archon_world_model::model::CpuLatentTransitionModel> {
    let registry = archon_world_model::registry::ModelRegistry::open(root).ok()?;
    let model_id = registry.active_model_id().ok().flatten()?;
    Some(registry.load_cpu_candidate(&model_id).ok()?.model)
}
