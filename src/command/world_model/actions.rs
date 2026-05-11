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
    scored.sort_by(|left, right| {
        let left_score = left.estimated_success - left.estimated_risk;
        let right_score = right.estimated_success - right.estimated_risk;
        right_score.total_cmp(&left_score)
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
