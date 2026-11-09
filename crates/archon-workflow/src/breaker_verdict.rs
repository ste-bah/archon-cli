//! Persisted diagnostics for requirement-directed perturbations.
use crate::breakage_proof::MutantKind;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
pub const VERDICT_SCHEMA: u32 = 1;
pub const BREAKER_LOGIC: u32 = 1;
pub const BREAKER_ATTEMPTS: u32 = 2;
pub const MAX_EQUIVALENCE_MARKS: usize = 2;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelIdentity {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    pub model: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoredMutant {
    pub mutant_id: String,
    pub kind: MutantKind,
    pub breaks: String,
    pub attempt: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttemptRecord {
    pub attempt: u32,
    pub reply_key: String,
    pub outcome: AttemptOutcome,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub refusals: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelevanceMark {
    pub mutant_id: String,
    pub check_id: String,
    pub relevance: Relevance,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub judge: ModelIdentity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cause: Option<UnavailableCause>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EquivalenceMark {
    pub mutant_id: String,
    pub first: JudgeAnswer,
    pub second: JudgeAnswer,
    pub marked: bool,
    pub capped: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JudgeAnswer {
    pub judge: ModelIdentity,
    pub answer: EquivalenceAnswer,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cause: Option<UnavailableCause>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SurvivorEvidence {
    pub mutant_id: String,
    pub kind: MutantKind,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checks: Vec<String>,
    pub patch: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    pub equivalence_unjudged: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwedSupplement {
    pub sup_id: String,
    pub requirement_id: String,
    pub task_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub survivors: Vec<SurvivorEvidence>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PairVerdict {
    pub task_id: String,
    pub id: String,
    pub verdict: Verdict,
    #[serde(default)]
    pub run_budget_exhausted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requirement_text_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mutants: Vec<AuthoredMutant>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attempts: Vec<AttemptRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relevance: Vec<RelevanceMark>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub equivalence: Vec<EquivalenceMark>,
    pub killed_by: BTreeMap<String, Vec<String>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub patch_keys: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owed: Option<OwedSupplement>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub survivors: Vec<SurvivorEvidence>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JudgeIdentities {
    pub breaker: ModelIdentity,
    pub relevance: ModelIdentity,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub equivalence: Vec<ModelIdentity>,
    pub equivalence_distinct: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProofVerdicts {
    pub schema: u32,
    pub base_commit: String,
    pub acceptance_digest: String,
    pub baseline_results_digest: String,
    pub snapshot_digest: String,
    pub rounds: u32,
    pub runs_executed: u64,
    pub runs_reused: u64,
    pub judges: JudgeIdentities,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_limits: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub refusals: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<PairVerdict>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    DoneCandidate,
    WeakTest,
    Unmutable,
    BodyGutUnavailable,
    NoViableBehaviourMutant,
    BreakerUnusable,
    UndecidedRelevance,
    UndecidedEquivalence,
    RequirementTextMissing,
    BreakerUnavailable,
    JudgeUnavailable,
    NoPassingCheck,
    NoCountableDeliverable,
    Unverified,
    BudgetExhausted,
    HostUnrun,
    Refused,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Relevance {
    Relevant,
    NotRelevant,
    QuoteUnverified,
    TouchesCheckInput,
    Unavailable,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EquivalenceAnswer {
    Equivalent,
    NotEquivalent,
    Unsure,
    Unavailable,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnavailableCause {
    Budget,
    Transient,
    Unusable,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum AttemptOutcome {
    Discarded {
        reason: String,
    },
    Accepted {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        mutants: Vec<String>,
    },
    Unusable {
        reason: String,
    },
    Unavailable {
        cause: UnavailableCause,
        reason: String,
    },
}
impl ModelIdentity {
    pub fn distinct(&self, other: &Self) -> bool {
        (self.provider_id.as_ref(), &self.model) != (other.provider_id.as_ref(), &other.model)
    }
}
#[path = "breaker_verdict_rule.rs"]
pub mod rule;
#[cfg(test)]
#[path = "breaker_verdict_tests.rs"]
mod tests;
