use std::path::Path;

use anyhow::{Result, bail};
use chrono::Utc;

use archon_world_model::embedding::{DeterministicHashEmbeddingAdapter, MemoryEmbeddingAdapter};
use archon_world_model::eval::{
    BrierImprovementReport, PromotionGateReport, evaluate_auxiliary_label_brier,
    evaluate_brier_improvement, evaluate_next_state_cosine_gate, evaluate_surprise_ks_gate,
};
use archon_world_model::jepa::{
    EvalRunStage, JEPA_MODEL_KIND, JepaEvalRecord, JepaPromotionGateReport,
    JepaRepresentationComparisonReport, PersistedEvalMode,
};
use archon_world_model::model::{CpuLatentTransitionModel, LatentTransitionExample};
use archon_world_model::registry::{CandidateEvalRecord, JepaCandidateRecord, ModelRegistry};
use archon_world_model::representation::GenericEmbeddingRepresentationAdapter;
use archon_world_model::schema::WorldLabelSet;
use archon_world_model::storage::WorldModelStore;

use super::embedding_runtime::build_embedding_adapter;

include!("00_train_eval/00_train_eval_core.rs");
include!("00_train_eval/01_eval_jepa_options.rs");
include!("00_train_eval/02_eval_jepa_full.rs");
