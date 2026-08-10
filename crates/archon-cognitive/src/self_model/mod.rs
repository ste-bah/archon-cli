mod rq_preflight;
mod store;
mod types;
mod writer;

pub use rq_preflight::{PreflightRecommendation, ReasoningQualityPreflight, RiskFlag, RiskReport};
pub use store::SelfModelStore;
pub use types::{
    ConfidenceCalibration, DomainTrust, FactKind, FailureCluster, MemoryContext, SelfModelBriefing,
    SelfModelFact, SelfModelProfile,
};
pub use writer::{MAX_CONFIDENCE_DRIFT, MIN_EVIDENCE_FOR_FACT, SelfModelUpdate, SelfModelWriter};
