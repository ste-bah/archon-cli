//! Local latent world-model types and scaffolding.
//!
//! M1 keeps this crate deliberately conservative: typed trace rows,
//! deterministic labels, backend metadata, eval thresholds, and advisor
//! surfaces. Training and runtime integration build on these stable contracts.

pub mod advisor;
pub mod artifact_ingest;
pub mod backend;
pub mod checkpoint;
pub mod counterfactual;
pub mod embedding;
pub mod embedding_audit;
pub mod eval;
pub mod evolution;
pub mod features;
pub mod guardrail;
pub mod ingest;
pub mod integration;
pub mod jepa;
pub mod labeler;
pub mod labels;
pub mod model;
pub mod registry;
pub mod representation;
pub mod schema;
pub mod shadow;
pub mod storage;
pub mod trace;
pub mod train;
pub mod trainer;

pub use advisor::{
    WorldAdvisor, WorldAdvisorConfig, WorldAdvisorContext, WorldAdvisorDecision,
    WorldAdvisorUnavailable, WorldAdvisorUnavailableReason, WorldPrediction,
};
pub use backend::{BackendKind, BackendStatus};
pub use eval::{EvalConfig, PromotionGateReport};
pub use guardrail::{
    GuardedActionKind, GuardrailFinalStatus, GuardrailReasonCode, GuardrailRequiredAction,
    GuardrailRiskScores, GuardrailStatusCounts, RuntimeTaskClass, VerificationKind,
    VerificationOutcome, VerificationRequirement, VerificationStatus, WorldGuardedAction,
    WorldGuardrailDecision, WorldGuardrailMode, WorldGuardrailOutcome, WorldGuardrailPolicyConfig,
    WorldGuardrailPredictionContext, WorldRiskTier,
};
pub use jepa::{
    JEPA_MODEL_KIND, JepaEvalRecord, JepaPromotionGateReport, JepaRepresentationComparisonReport,
    JepaTraceModel, JepaTrainingConfig, JepaTrainingExample, JepaTrainingLosses,
    JepaTrainingOutcome,
};
pub use representation::{
    GenericEmbeddingRepresentationAdapter, TraceAction, TraceTransition, TraceWindow,
    TraceWindowBuilder, WorldRepresentationAdapter,
};
pub use schema::{EvidenceRef, WorldLabelSet, WorldTraceRow};
pub use trace::{ColdStartStats, ColdStartStatus, ColdStartThresholds};
