pub mod candidate_planner;
mod candidate_store;
pub mod config;
mod cozo_guard;
mod decision_codec;
pub mod decision_store;
pub mod executive_loop;
mod executive_support;
mod governed_apply_store;
pub mod governed_autonomous_apply;
pub mod policy_gate;
mod reflection_store;
pub mod reflection_writer;
pub mod schema;
pub mod self_model;
pub mod situation_classifier;
pub mod store;
pub mod tool_use_gate;
pub mod types;
pub mod verification_contracts;
pub mod world_model_scoring;

pub use candidate_planner::{CandidatePlanner, HeuristicWeights};
pub use config::CognitiveConfig;
pub use decision_store::DecisionStore;
pub use executive_loop::{
    ActionExecution, ActionExecutor, ActionOutcome, ExecutiveLoop, ExecutiveRunOutcome,
    ExecutiveTurnInput, NoopActionExecutor, PlannedActionInput,
};
pub use governed_autonomous_apply::{
    ApplyResult, BehaviourManifestKind, CanaryOutcome, GovernedAutonomousApply, Proposal,
};
pub use policy_gate::{DenyReason, PolicyGate, PolicyVerdict, ProposalCheck, ProposalDenyReason};
pub use reflection_writer::{
    LessonSink, NoopLessonSink, OutcomeSummary, ReflectInput, ReflectionRecord,
    ReflectionWriteOutcome, ReflectionWriter,
};
pub use schema::{CURRENT_SCHEMA_VERSION, cognitive_schema_version, ensure_cognitive_schema};
pub use situation_classifier::{ClassifyInput, SituationClassifier};
pub use store::{CognitiveStore, PersistentCognitiveStore};
pub use tool_use_gate::{ToolGateInput, ToolUseGate};
pub use types::{
    Candidate, CandidateActionKind, CandidateScore, ClassifierConfidence, CognitiveDecision,
    CognitiveError, CognitiveSurface, DecisionRecord, ExecutiveStateSnapshot, RejectedCandidate,
    RiskLevel, ScoreSource, Situation, SituationKind, ToolVerdict, direct_response_for,
};
pub use verification_contracts::{
    ContractInput, VerificationContract, VerificationEngine, VerificationEvidence,
    VerificationKind, VerificationRequirement, VerificationVerdict,
};
pub use world_model_scoring::{
    ModelKind, ModelPrediction, PredictionBackend, PredictionDimensions, ScoredCandidates,
    WorldModelScorer, WorldModelState,
};
