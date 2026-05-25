pub mod candidate_planner;
mod candidate_store;
pub mod config;
mod cozo_guard;
mod decision_codec;
pub mod decision_store;
pub mod schema;
pub mod self_model;
pub mod situation_classifier;
pub mod store;
pub mod tool_use_gate;
pub mod types;
pub mod verification_contracts;

pub use candidate_planner::{CandidatePlanner, HeuristicWeights};
pub use config::CognitiveConfig;
pub use decision_store::DecisionStore;
pub use schema::{CURRENT_SCHEMA_VERSION, cognitive_schema_version, ensure_cognitive_schema};
pub use situation_classifier::{ClassifyInput, SituationClassifier};
pub use store::{CognitiveStore, PersistentCognitiveStore};
pub use tool_use_gate::{ToolGateInput, ToolUseGate};
pub use types::{
    Candidate, CandidateActionKind, CandidateScore, ClassifierConfidence, CognitiveDecision,
    CognitiveError, CognitiveSurface, DecisionRecord, RejectedCandidate, RiskLevel, ScoreSource,
    Situation, SituationKind, ToolVerdict, direct_response_for,
};
pub use verification_contracts::{
    ContractInput, VerificationContract, VerificationEngine, VerificationEvidence,
    VerificationKind, VerificationRequirement, VerificationVerdict,
};
