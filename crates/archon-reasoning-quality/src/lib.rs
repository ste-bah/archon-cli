//! First-class reasoning-quality events for visible agent claims.
//!
//! This crate owns text-level claim/evidence signal. It deliberately records
//! observable assistant output and chronology, not hidden chain-of-thought.

pub mod audit;
pub mod canonical;
pub mod correction;
pub mod critic;
pub mod evidence;
pub mod extractor;
pub mod fixtures;
pub mod patterns;
pub mod redaction;
pub mod severity;
pub mod store;
pub mod types;

pub use audit::{FixtureAuditFinding, FixtureAuditReport, audit_labeled_turns};
pub use canonical::{
    CANONICALIZER_VERSION, canonicalize_claim_text, claim_id_for, event_id_for, hash_hex,
};
pub use correction::build_user_correction_event;
pub use critic::{
    CriticBudgetDecision, CriticBudgetLimits, CriticBudgetUsage, CriticCoverage, CriticFinding,
    check_critic_budget, coverage_for, parse_critic_response,
};
pub use evidence::{EvidenceMatch, build_superseding_source_events, match_claim_evidence};
pub use extractor::{DeterministicExtractor, ExtractorConfig};
pub use fixtures::{FixtureEvaluation, LabeledTurnFixture, evaluate_labeled_turns};
pub use patterns::{RepeatedReasoningPattern, detect_repeated_patterns};
pub use redaction::{RedactionConfig, redact_entity_key, redact_text};
pub use severity::{SeverityOverride, base_severity, effective_severity};
pub use types::{
    BridgeName, BriefingApplicability, ConfidenceSignal, DataFlowClass, EvidenceKind, EvidenceRef,
    ReasoningClaim, ReasoningEventKind, ReasoningQualityEvent, ReasoningSubject,
    ReasoningTurnInput, VerificationState,
};
