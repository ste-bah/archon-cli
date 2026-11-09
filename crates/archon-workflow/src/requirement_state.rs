//! Host-owned decisions and portable evidence carried by the frozen record.
use crate::breaker_verdict::{PairVerdict, Verdict};
use crate::v2::requirement_classes::{Fallback, RequirementClass, RequirementEntry};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequirementState {
    Done,
    Missing,
    Undecided,
    Inspection,
}
impl RequirementState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Done => "done",
            Self::Missing => "missing",
            Self::Undecided => "undecided",
            Self::Inspection => "inspection",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequirementDecision {
    pub state: RequirementState,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}
impl RequirementDecision {
    pub fn new(state: RequirementState, reason: impl Into<String>) -> Self {
        Self {
            state,
            reason: reason.into(),
            notes: Vec::new(),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DoneKind {
    Breakage,
    TextEvidence,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoneProof {
    pub id: String,
    pub kind: DoneKind,
    pub check_id: String,
    pub check_digest: String,
    pub base_commit: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mutant_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_gut_mutant_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub patch_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub patch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub literals: Vec<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub evidence_truncated: bool,
}
pub const CLASSIFICATION_PREFIX: &str = "Host classification (frozen, proven on the base tree):";
pub const DONE_EVIDENCE_BYTES_MAX: usize = 1024 * 1024;
#[derive(Debug, Default)]
pub struct DecisionOutcome {
    pub decisions: BTreeMap<(String, String), RequirementDecision>,
    pub proofs: BTreeMap<(String, String), DoneProof>,
    pub undecided: Vec<UndecidedNote>,
}
#[derive(Debug)]
pub struct UndecidedNote {
    pub task_id: String,
    pub id: String,
    pub reason: String,
    pub survivors: Vec<String>,
    pub equivalence_unjudged: usize,
}
pub struct DecisionContext<'a> {
    pub inspection: bool,
    pub disabled: bool,
    pub refusal: Option<&'a str>,
    pub text_evidence: bool,
}
pub fn decide(
    entry: &RequirementEntry,
    verdict: Option<&PairVerdict>,
    ctx: DecisionContext<'_>,
) -> RequirementDecision {
    use RequirementState::*;
    let base_failing = entry.fallback == Some(Fallback::Missing)
        || entry
            .checks
            .values()
            .any(|result| *result == Some(crate::baseline_results::BaselineOutcome::Failed));
    let (state, reason) = match entry.class {
        RequirementClass::NoCheck if ctx.inspection => (Inspection, "inspection_pending"),
        RequirementClass::NoCheck => (Missing, "no_check"),
        RequirementClass::Missing => (
            Missing,
            entry.reason.as_deref().unwrap_or("all_checks_fail_on_base"),
        ),
        RequirementClass::NoBaseResult if base_failing => (Missing, "base_failing_check"),
        RequirementClass::NoBaseResult => (Undecided, "no_base_result"),
        RequirementClass::CandidateDone => {
            if base_failing && (ctx.disabled || ctx.refusal.is_some()) {
                return RequirementDecision::new(Missing, "base_failing_check");
            }
            if ctx.disabled {
                return RequirementDecision::new(Undecided, "proof_disabled");
            }
            if let Some(code) = ctx.refusal {
                return RequirementDecision::new(
                    Undecided,
                    if code == "proof_site_unavailable" {
                        code.into()
                    } else {
                        format!("proof_refused:{code}")
                    },
                );
            }
            if verdict.is_some_and(|v| v.verdict == Verdict::DoneCandidate) {
                (Done, "breakage_kill")
            } else if ctx.text_evidence
                && entry.text_item
                && verdict.is_none_or(|v| {
                    v.survivors.is_empty()
                        && matches!(
                            v.verdict,
                            Verdict::Unmutable | Verdict::NoCountableDeliverable
                        )
                })
            {
                (Done, "text_evidence")
            } else if base_failing {
                (Missing, "base_failing_check")
            } else {
                match verdict.map(|v| v.verdict) {
                    Some(Verdict::DoneCandidate) => unreachable!(),
                    Some(Verdict::WeakTest) => (Undecided, "weak_test"),
                    Some(Verdict::Unmutable) => (Missing, "unmutable"),
                    Some(Verdict::BodyGutUnavailable) => (Undecided, "body_gut_unavailable"),
                    Some(Verdict::NoViableBehaviourMutant) => {
                        (Undecided, "no_viable_behaviour_mutant")
                    }
                    Some(Verdict::BreakerUnusable) => (Undecided, "breaker_unusable"),
                    Some(Verdict::UndecidedRelevance) => (Undecided, "irrelevant_kill"),
                    Some(Verdict::UndecidedEquivalence) => (Undecided, "weak_test_undecided"),
                    Some(Verdict::RequirementTextMissing) => {
                        (Undecided, "requirement_text_missing")
                    }
                    Some(Verdict::BreakerUnavailable | Verdict::JudgeUnavailable) => (
                        Undecided,
                        if verdict.is_some_and(author_unusable) {
                            "author_unusable"
                        } else {
                            "author_unavailable"
                        },
                    ),
                    Some(Verdict::NoPassingCheck) => (Undecided, "no_passing_check"),
                    Some(Verdict::NoCountableDeliverable) => (Missing, "no_countable_deliverable"),
                    Some(Verdict::Unverified) => (Undecided, "control_unstable"),
                    Some(Verdict::BudgetExhausted) => (Undecided, "budget_exhausted"),
                    Some(Verdict::HostUnrun) | None => (Undecided, "host_unrun"),
                    Some(Verdict::Refused) => (Undecided, "proof_refused:unspecified"),
                }
            }
        }
    };
    RequirementDecision::new(state, reason)
}
pub fn unavailable_causes(
    pair: &PairVerdict,
) -> impl Iterator<Item = crate::breaker_verdict::UnavailableCause> + '_ {
    use crate::breaker_verdict::AttemptOutcome;
    pair.attempts
        .iter()
        .filter_map(|a| match a.outcome {
            AttemptOutcome::Unavailable { cause, .. } => Some(cause),
            _ => None,
        })
        .chain(pair.relevance.iter().filter_map(|m| m.cause))
        .chain(
            pair.equivalence
                .iter()
                .flat_map(|m| [m.first.cause, m.second.cause].into_iter().flatten()),
        )
}
pub fn author_unusable(pair: &PairVerdict) -> bool {
    use crate::breaker_verdict::UnavailableCause;
    !unavailable_causes(pair).any(|c| c == UnavailableCause::Transient)
        && unavailable_causes(pair).any(|c| c == UnavailableCause::Unusable)
}
pub fn inspection_only(text: &str, constraint: bool) -> bool {
    constraint
        && !text.contains('`')
        && !text.split_whitespace().any(|word| {
            if word.contains(['/', '\\']) {
                return true;
            }
            let word = word
                .trim_matches(|c: char| c.is_ascii_punctuation() && c != '.')
                .trim_end_matches(['.', ',', ';', ':', '!', '?']);
            word.rsplit_once('.').is_some_and(|(name, ext)| {
                !name.is_empty()
                    && (1..=10).contains(&ext.len())
                    && ext.as_bytes()[0].is_ascii_alphabetic()
                    && ext.bytes().all(|b| b.is_ascii_alphanumeric())
            })
        })
}
pub fn task_status(states: &BTreeMap<String, RequirementDecision>) -> Option<String> {
    if states.is_empty()
        || states.values().any(|d| {
            matches!(
                d.state,
                RequirementState::Missing | RequirementState::Inspection
            )
        })
    {
        None
    } else if states.values().all(|d| d.state == RequirementState::Done) {
        Some("done".into())
    } else {
        Some("kept".into())
    }
}
pub fn bound_evidence(proofs: &mut [DoneProof]) {
    let bytes = |p: &DoneProof| {
        [&p.patch, &p.output_tail]
            .into_iter()
            .filter_map(Option::as_ref)
            .map(|s| serde_json::to_vec(s).expect("serializable evidence").len())
            .sum::<usize>()
    };
    let mut total: usize = proofs.iter().map(bytes).sum();
    let mut order: Vec<_> = (0..proofs.len()).collect();
    order.sort_by_key(|i| std::cmp::Reverse(bytes(&proofs[*i])));
    for i in order {
        if total <= DONE_EVIDENCE_BYTES_MAX {
            break;
        }
        total -= bytes(&proofs[i]);
        proofs[i].patch = None;
        proofs[i].output_tail = None;
        proofs[i].evidence_truncated = true;
    }
}
#[cfg(test)]
#[path = "requirement_state_tests.rs"]
mod tests;

impl DecisionOutcome {
    pub fn apply_to(&self, skeleton: &mut crate::task_skeleton::TaskSkeleton) {
        let mut proofs: Vec<_> = self.proofs.values().cloned().collect();
        bound_evidence(&mut proofs);
        let proofs: std::collections::BTreeMap<_, _> =
            self.proofs.keys().cloned().zip(proofs).collect();
        for task in &mut skeleton.tasks {
            task.requirement_states = self
                .decisions
                .iter()
                .filter(|((t, _), _)| t == &task.task_id)
                .map(|((_, id), d)| (id.clone(), d.clone()))
                .collect();
            task.proven_done = proofs
                .iter()
                .filter(|((owner, _), _)| owner == &task.task_id)
                .map(|(_, p)| p.clone())
                .collect();
            task.status = task_status(&task.requirement_states);
        }
    }
}
