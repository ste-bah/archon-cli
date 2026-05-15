use crate::backend::BackendKind;
use crate::embedding::{
    DeterministicHashEmbeddingAdapter, EmbeddingRequest, WorldEmbeddingAdapter,
};
use crate::features::graph_context_for_row;
use crate::model::{CpuLatentTransitionModel, LatentTransitionExample, LatentWorldModelMetadata};
use crate::representation::{TraceWindowBuilder, WorldRepresentationAdapter};
use crate::schema::WorldTraceRow;
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrainingConfig {
    pub backend: BackendKind,
    pub allow_cpu_fallback: bool,
    pub prefer_accelerator: bool,
    pub precision: String,
    pub max_accelerator_memory_mb: u64,
    pub batch_size: usize,
    pub max_epochs: usize,
    pub validation_split: f32,
    pub promotion_min_delta: f32,
    pub max_runtime_ms: u64,
}

impl Default for TrainingConfig {
    fn default() -> Self {
        Self {
            backend: BackendKind::Auto,
            allow_cpu_fallback: true,
            prefer_accelerator: true,
            precision: "fp32".into(),
            max_accelerator_memory_mb: 4_096,
            batch_size: 32,
            max_epochs: 10,
            validation_split: 0.2,
            promotion_min_delta: 0.02,
            max_runtime_ms: 300_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrainingStatus {
    NotStarted,
    CandidateWritten,
    Rejected,
    Promoted,
}

impl Default for TrainingStatus {
    fn default() -> Self {
        Self::NotStarted
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrainingOutcome {
    pub status: TrainingStatus,
    pub metadata: LatentWorldModelMetadata,
    pub training_mean_cosine_error: f32,
}

pub fn train_cpu_candidate(
    state_dim: usize,
    examples: &[LatentTransitionExample],
) -> Result<(CpuLatentTransitionModel, TrainingOutcome)> {
    train_candidate_with_backend(state_dim, examples, BackendKind::Cpu)
}

pub fn train_candidate_with_backend(
    state_dim: usize,
    examples: &[LatentTransitionExample],
    backend: BackendKind,
) -> Result<(CpuLatentTransitionModel, TrainingOutcome)> {
    if matches!(backend, BackendKind::Cuda | BackendKind::Metal) {
        let probe = crate::backend::probe_backend(backend);
        if !probe.available {
            anyhow::bail!(
                "{backend} backend probe failed: {}",
                probe.reason.as_deref().unwrap_or("unknown")
            );
        }
    }
    let model = match backend {
        BackendKind::Cuda => {
            crate::backend::candle::candle_cuda_fit_transition_model(state_dim, examples)?
        }
        BackendKind::Metal => {
            crate::backend::mlx::mlx_metal_fit_transition_model(state_dim, examples)?
        }
        BackendKind::Auto | BackendKind::Cpu => {
            crate::backend::candle::candle_cpu_fit_transition_model(state_dim, examples)?
        }
    };
    let training_mean_cosine_error = model.mean_cosine_error(examples)?;
    let outcome = TrainingOutcome {
        status: TrainingStatus::CandidateWritten,
        metadata: model.metadata.clone(),
        training_mean_cosine_error,
    };

    Ok((model, outcome))
}

pub fn train_candidate_with_backend_or_cpu_fallback(
    state_dim: usize,
    examples: &[LatentTransitionExample],
    backend: BackendKind,
    allow_cpu_fallback: bool,
) -> Result<(CpuLatentTransitionModel, TrainingOutcome)> {
    match train_candidate_with_backend(state_dim, examples, backend) {
        Ok(candidate) => Ok(candidate),
        Err(error) if backend != BackendKind::Cpu && allow_cpu_fallback => {
            train_candidate_with_backend(state_dim, examples, BackendKind::Cpu)
                .map_err(|fallback_error| {
                    anyhow::anyhow!(
                        "accelerator training failed: {error}; cpu fallback failed: {fallback_error}"
                    )
                })
        }
        Err(error) => Err(error),
    }
}

pub fn examples_from_rows(
    rows: &[WorldTraceRow],
    state_dim: usize,
) -> Result<Vec<LatentTransitionExample>> {
    let adapter = DeterministicHashEmbeddingAdapter::new(state_dim)?;
    examples_from_rows_with_adapter(rows, &adapter)
}

pub fn examples_from_rows_with_adapter(
    rows: &[WorldTraceRow],
    adapter: &dyn WorldEmbeddingAdapter,
) -> Result<Vec<LatentTransitionExample>> {
    let mut sorted = rows.to_vec();
    sorted.sort_by(|left, right| {
        left.session_id
            .cmp(&right.session_id)
            .then_with(|| left.created_at.cmp(&right.created_at))
            .then_with(|| left.row_id.cmp(&right.row_id))
    });

    let mut examples = Vec::new();
    for window in sorted.windows(2) {
        let current = &window[0];
        let next = &window[1];
        if current.session_id != next.session_id {
            continue;
        }

        examples.push(LatentTransitionExample {
            state: embed_row(adapter, &sorted, current, "state")?,
            action: embed_row(adapter, &sorted, current, "action")?,
            next_state: embed_row(adapter, &sorted, next, "next_state")?,
            labels: next.labels.clone(),
        });
    }

    Ok(examples)
}

pub fn examples_from_rows_with_representation_adapter(
    rows: &[WorldTraceRow],
    adapter: &dyn WorldRepresentationAdapter,
) -> Result<Vec<LatentTransitionExample>> {
    let builder = TraceWindowBuilder::new(rows);
    let transitions = builder.adjacent_transitions(1, 1, 1)?;
    transitions
        .into_iter()
        .map(|transition| {
            Ok(LatentTransitionExample {
                state: adapter.encode_state(&transition.context)?,
                action: adapter.encode_action(&transition.action)?,
                next_state: adapter.encode_target(&transition.target)?,
                labels: transition.labels,
            })
        })
        .collect()
}

fn embed_row(
    adapter: &dyn WorldEmbeddingAdapter,
    rows: &[WorldTraceRow],
    row: &WorldTraceRow,
    role: &str,
) -> Result<Vec<f32>> {
    let request = EmbeddingRequest {
        text: row_embedding_text(rows, row, role),
        source_hash: row.row_id.clone(),
        redaction_policy: "world-model-default-redacted".into(),
    };
    Ok(adapter.embed(&request)?.values)
}

fn row_embedding_text(rows: &[WorldTraceRow], row: &WorldTraceRow, role: &str) -> String {
    let graph = graph_context_for_row(rows, row).compact_text();
    format!(
        "{role} source={:?} action={:?} provider={} model={} agent={} {graph} text={}",
        row.source,
        row.action_kind,
        row.provider.as_deref().unwrap_or(""),
        row.model.as_deref().unwrap_or(""),
        row.agent.as_deref().unwrap_or(""),
        row.redacted_excerpt.as_deref().unwrap_or("")
    )
}

#[cfg(test)]
mod tests {
    use crate::model::LatentTransitionExample;
    use crate::representation::GenericEmbeddingRepresentationAdapter;
    use crate::schema::{WorldActionKind, WorldTraceRow};

    use super::*;

    #[test]
    fn cpu_training_writes_candidate_outcome() {
        let examples = [LatentTransitionExample {
            state: vec![0.0, 0.0],
            action: vec![0.0, 0.0],
            next_state: vec![1.0, 1.0],
            labels: Default::default(),
        }];

        let (model, outcome) = train_cpu_candidate(2, &examples).unwrap();

        assert_eq!(model.metadata.backend, BackendKind::Cpu);
        assert_eq!(outcome.status, TrainingStatus::CandidateWritten);
        assert_eq!(outcome.metadata.row_count, 1);
        assert!(outcome.training_mean_cosine_error <= 1.0);
    }

    #[test]
    fn accelerator_training_can_fallback_to_cpu() {
        let Some(backend) = [BackendKind::Cuda, BackendKind::Metal]
            .into_iter()
            .find(|backend| !crate::backend::probe_backend(*backend).available)
        else {
            return;
        };

        let examples = [LatentTransitionExample {
            state: vec![0.0, 0.0],
            action: vec![0.0, 0.0],
            next_state: vec![1.0, 1.0],
            labels: Default::default(),
        }];

        let (model, _) =
            train_candidate_with_backend_or_cpu_fallback(2, &examples, backend, true).unwrap();

        assert_eq!(model.metadata.backend, BackendKind::Cpu);
    }

    #[test]
    fn examples_from_rows_builds_session_transitions() {
        let mut first =
            WorldTraceRow::new("session-1", WorldActionKind::ToolCall).with_row_id("r1");
        first.redacted_excerpt = Some("run tests".into());
        let mut second =
            WorldTraceRow::new("session-1", WorldActionKind::Verification).with_row_id("r2");
        second.redacted_excerpt = Some("tests passed".into());

        let examples = examples_from_rows(&[second, first], 8).unwrap();

        assert_eq!(examples.len(), 1);
        assert_eq!(examples[0].state.len(), 8);
        assert_eq!(examples[0].action.len(), 8);
        assert_eq!(examples[0].next_state.len(), 8);
    }

    #[test]
    fn examples_from_rows_accepts_custom_embedding_adapter() {
        let mut first =
            WorldTraceRow::new("session-1", WorldActionKind::ToolCall).with_row_id("r1");
        first.redacted_excerpt = Some("run tests".into());
        let mut second =
            WorldTraceRow::new("session-1", WorldActionKind::Verification).with_row_id("r2");
        second.redacted_excerpt = Some("tests passed".into());
        let adapter = DeterministicHashEmbeddingAdapter::new(4).unwrap();

        let examples = examples_from_rows_with_adapter(&[first, second], &adapter).unwrap();

        assert_eq!(examples[0].state.len(), 4);
    }

    #[test]
    fn examples_from_rows_accepts_representation_adapter() {
        let mut first =
            WorldTraceRow::new("session-1", WorldActionKind::ToolCall).with_row_id("r1");
        first.redacted_excerpt = Some("run tests".into());
        let mut second =
            WorldTraceRow::new("session-1", WorldActionKind::Verification).with_row_id("r2");
        second.redacted_excerpt = Some("tests passed".into());
        let adapter = GenericEmbeddingRepresentationAdapter::new(Box::new(
            DeterministicHashEmbeddingAdapter::new(4).unwrap(),
        ));

        let examples =
            examples_from_rows_with_representation_adapter(&[second, first], &adapter).unwrap();

        assert_eq!(examples.len(), 1);
        assert_eq!(examples[0].state.len(), 4);
        assert_eq!(examples[0].action.len(), 4);
        assert_eq!(examples[0].next_state.len(), 4);
    }
}
