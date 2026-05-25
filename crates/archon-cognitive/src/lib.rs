pub mod config;
pub mod schema;
pub mod situation_classifier;
pub mod store;
pub mod tool_use_gate;
pub mod types;

pub use config::CognitiveConfig;
pub use schema::{CURRENT_SCHEMA_VERSION, cognitive_schema_version, ensure_cognitive_schema};
pub use situation_classifier::{ClassifyInput, SituationClassifier};
pub use store::{CognitiveStore, PersistentCognitiveStore};
pub use tool_use_gate::{ToolGateInput, ToolUseGate};
pub use types::{
    ClassifierConfidence, CognitiveDecision, CognitiveError, CognitiveSurface, Situation,
    SituationKind, ToolVerdict, direct_response_for,
};
