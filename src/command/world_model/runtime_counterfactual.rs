use std::path::PathBuf;

use anyhow::Result;
use archon_world_model::counterfactual::{CounterfactualExample, CounterfactualScore};
use archon_world_model::embedding::{EmbeddingRequest, WorldEmbeddingAdapter};
use archon_world_model::schema::WorldTraceRow;
use archon_world_model::storage::WorldModelStore;
use chrono::{DateTime, Utc};
use serde::Serialize;

#[derive(Debug, Serialize)]
struct RuntimeCounterfactualRecord {
    record_id: String,
    surface: archon_world_model::integration::WorldAdvisorSurface,
    task: String,
    scores: Vec<CounterfactualScore>,
    shadow: archon_world_model::shadow::ShadowPlanReport,
    created_at: DateTime<Utc>,
}

pub(super) fn runtime_counterfactual_advice(
    config: &archon_core::config::ArchonConfig,
    surface: archon_world_model::integration::WorldAdvisorSurface,
    task: &str,
    choices: &[(&str, &str)],
) -> Result<PathBuf> {
    if choices.is_empty() {
        anyhow::bail!("runtime counterfactual advice requires candidate choices");
    }
    let root = super::super::world_model_root()?;
    let adapter = super::super::embedding_runtime::build_embedding_adapter(config)?;
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
