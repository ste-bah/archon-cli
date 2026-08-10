//! Scoring: how much each candidate is supported, and by what.
//!
//! The score is a sum of typed evidence, and the evidence list travels with the
//! candidate. That is the difference between "confidence 0.62" and a claim
//! somebody can argue with: an adjudicator reading the row can see that 0.45 of
//! it was a tool the provider itself marked as failed and 0.15 was recency, and
//! can disagree with the second without discarding the first.
//!
//! Two of the six kinds are weak on purpose. Recency and lexical overlap are
//! true of the wrong candidate about as often as the right one -- a correction
//! mentioning a filename matches every tool that touched that file, and the most
//! recent action is the most recent whether or not it caused anything. The
//! roadmap says it directly: "never infer ownership from lexical similarity
//! alone". So they contribute score but cannot carry an acceptance
//! ([`EvidenceKind::is_corroborating`]).

use crate::attribution::CauseActionClass;
use crate::attribution::candidates::{CausalCandidate, sole_tool_run};
use crate::attribution::input::{ActionEffectClass, AttributionInput};

/// Confidence at or above which a candidate may be accepted.
pub const ACCEPT_CONFIDENCE: f32 = 0.55;

/// How far the top candidate must beat the runner-up.
///
/// Without this a window of five equally-supported actions produces a confident
/// answer that is one-in-five right. A near-tie is not a weak answer, it is the
/// absence of one, and it abstains.
pub const AMBIGUITY_MARGIN: f32 = 0.10;

// ── evidence weights ─────────────────────────────────────────
//
// Deterministic evidence outranks everything, per the roadmap's global
// constraint. The ordering of these constants is the policy; their exact values
// are version 1 and move only with a version bump.

const WEIGHT_DETERMINISTIC_FAILURE: f32 = 0.45;
const WEIGHT_PERMISSION_BLOCK: f32 = 0.40;
const WEIGHT_EFFECT_AFFINITY: f32 = 0.35;
const WEIGHT_SOLE_ACTION: f32 = 0.30;
const WEIGHT_DECISION_SCOPE: f32 = 0.15;
const WEIGHT_RECENCY: f32 = 0.15;
const WEIGHT_LEXICAL: f32 = 0.20;

/// Why a candidate is supported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvidenceKind {
    /// The provider marked the tool result an error.
    DeterministicFailure,
    /// The run was refused before executing.
    PermissionBlock,
    /// The correction type and the action's effect class match: a
    /// permission complaint about an action that could change something.
    EffectClassAffinity,
    /// The eligible window contains exactly one tool run.
    SoleEligibleAction,
    /// The candidate is the decision the corrected turn was taken under.
    DecisionScope,
    /// The candidate is close to the correction in turns and ordinal.
    Recency,
    /// The correction's words overlap the action's name and inputs.
    LexicalOverlap,
}

impl EvidenceKind {
    pub fn as_code(self) -> &'static str {
        match self {
            Self::DeterministicFailure => "deterministic_failure",
            Self::PermissionBlock => "permission_block",
            Self::EffectClassAffinity => "effect_class_affinity",
            Self::SoleEligibleAction => "sole_eligible_action",
            Self::DecisionScope => "decision_scope",
            Self::Recency => "recency",
            Self::LexicalOverlap => "lexical_overlap",
        }
    }

    /// Whether this kind can support an acceptance on its own.
    ///
    /// Recency and lexical overlap cannot. They are properties of the window and
    /// of the words, not of what happened, and an attribution resting only on
    /// them is the "plausible but wrong" outcome that is worse than none because
    /// it becomes training data.
    pub fn is_corroborating(self) -> bool {
        !matches!(self, Self::Recency | Self::LexicalOverlap)
    }
}

/// One supported reason, with the weight it contributed.
#[derive(Debug, Clone, PartialEq)]
pub struct CausalEvidence {
    pub kind: EvidenceKind,
    pub weight: f32,
    pub detail: String,
}

/// A candidate with its score and the evidence behind it.
#[derive(Debug, Clone, PartialEq)]
pub struct ScoredCandidate {
    pub candidate: CausalCandidate,
    pub confidence: f32,
    pub evidence: Vec<CausalEvidence>,
    /// 0-based rank in the scored population.
    pub rank: usize,
}

impl ScoredCandidate {
    /// Whether anything other than recency and word overlap supports this.
    pub fn has_corroboration(&self) -> bool {
        self.evidence
            .iter()
            .any(|evidence| evidence.kind.is_corroborating())
    }

    pub fn evidence_codes(&self) -> Vec<&'static str> {
        self.evidence
            .iter()
            .map(|evidence| evidence.kind.as_code())
            .collect()
    }
}

/// Correction types whose complaint is about an action's effect.
fn effect_affinity(correction_type_code: &str, effect: ActionEffectClass) -> bool {
    matches!(
        (correction_type_code, effect),
        (
            "acted_without_permission" | "did_forbidden_action",
            ActionEffectClass::Mutate
        ) | ("factual_error", ActionEffectClass::Read)
    )
}

/// Correction types whose complaint is about the choice rather than the act.
fn decision_affinity(correction_type_code: &str) -> bool {
    matches!(
        correction_type_code,
        "approach_correction" | "repeated_instruction"
    )
}

/// Tokens used for lexical overlap.
///
/// Short and very common words are dropped: they match everything, which is the
/// same as matching nothing while still adding score.
fn tokens(text: &str) -> std::collections::BTreeSet<String> {
    const COMMON: &[&str] = &[
        "a", "an", "and", "are", "did", "dont", "for", "from", "have", "not", "should", "that",
        "the", "them", "then", "there", "this", "was", "were", "with", "you", "your",
    ];
    text.split(|character: char| !character.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|token| token.len() > 2 && !COMMON.contains(&token.as_str()))
        .collect()
}

fn recency_weight(candidate: &CausalCandidate, correction_turn: u64, newest_ordinal: u32) -> f32 {
    let turn_factor = match candidate.turn_distance(correction_turn) {
        0 | 1 => 1.0,
        _ => 0.5,
    };
    let ordinal_factor = if candidate.ordinal >= newest_ordinal {
        1.0
    } else {
        0.6
    };
    WEIGHT_RECENCY * turn_factor * ordinal_factor
}

fn lexical_weight(correction_tokens: &std::collections::BTreeSet<String>, text: &str) -> f32 {
    let candidate_tokens = tokens(text);
    if correction_tokens.is_empty() || candidate_tokens.is_empty() {
        return 0.0;
    }
    let overlap = correction_tokens.intersection(&candidate_tokens).count() as f32;
    let ratio = overlap / candidate_tokens.len().min(correction_tokens.len()) as f32;
    WEIGHT_LEXICAL * ratio.min(1.0)
}

/// Score every candidate, highest first.
pub fn score(input: &AttributionInput, candidates: Vec<CausalCandidate>) -> Vec<ScoredCandidate> {
    let correction_turn = input.correction.turn_number;
    let type_code = input.correction.correction_type_code.as_str();
    let correction_tokens = tokens(&input.correction.summary);
    let sole = sole_tool_run(&candidates).map(str::to_string);
    let newest_ordinal = candidates
        .iter()
        .filter(|candidate| candidate.turn_distance(correction_turn) <= 1)
        .map(|candidate| candidate.ordinal)
        .max()
        .unwrap_or(0);

    let mut scored: Vec<ScoredCandidate> = candidates
        .into_iter()
        .map(|candidate| {
            let mut evidence = Vec::new();

            if candidate.failed {
                evidence.push(CausalEvidence {
                    kind: EvidenceKind::DeterministicFailure,
                    weight: WEIGHT_DETERMINISTIC_FAILURE,
                    detail: format!("{} returned an error result", candidate.label),
                });
            }
            if candidate.blocked {
                evidence.push(CausalEvidence {
                    kind: EvidenceKind::PermissionBlock,
                    weight: WEIGHT_PERMISSION_BLOCK,
                    detail: format!("{} was refused before running", candidate.label),
                });
            }
            match candidate.cause_action_class {
                CauseActionClass::ToolRun => {
                    if effect_affinity(type_code, candidate.effect_class) {
                        evidence.push(CausalEvidence {
                            kind: EvidenceKind::EffectClassAffinity,
                            weight: WEIGHT_EFFECT_AFFINITY,
                            detail: format!(
                                "{type_code} matches a {} action",
                                candidate.effect_class.as_code()
                            ),
                        });
                    }
                    if sole.as_deref() == Some(candidate.candidate_id.as_str()) {
                        evidence.push(CausalEvidence {
                            kind: EvidenceKind::SoleEligibleAction,
                            weight: WEIGHT_SOLE_ACTION,
                            detail: "the only tool run in the eligible window".to_string(),
                        });
                    }
                }
                CauseActionClass::Decision => {
                    evidence.push(CausalEvidence {
                        kind: EvidenceKind::DecisionScope,
                        weight: WEIGHT_DECISION_SCOPE,
                        detail: "the decision the corrected turn was taken under".to_string(),
                    });
                    if decision_affinity(type_code) {
                        evidence.push(CausalEvidence {
                            kind: EvidenceKind::EffectClassAffinity,
                            weight: WEIGHT_EFFECT_AFFINITY,
                            detail: format!("{type_code} is a complaint about the choice made"),
                        });
                    }
                }
            }

            let recency = recency_weight(&candidate, correction_turn, newest_ordinal);
            if recency > 0.0 {
                evidence.push(CausalEvidence {
                    kind: EvidenceKind::Recency,
                    weight: recency,
                    detail: format!(
                        "{} turn(s) before the correction",
                        candidate.turn_distance(correction_turn)
                    ),
                });
            }
            let lexical = lexical_weight(&correction_tokens, &candidate.text);
            if lexical > 0.0 {
                evidence.push(CausalEvidence {
                    kind: EvidenceKind::LexicalOverlap,
                    weight: lexical,
                    detail: "correction wording overlaps the action".to_string(),
                });
            }

            let confidence = evidence
                .iter()
                .map(|item| item.weight)
                .sum::<f32>()
                .clamp(0.0, 1.0);
            ScoredCandidate {
                candidate,
                confidence,
                evidence,
                rank: 0,
            }
        })
        .collect();

    // Total order: confidence, then the candidate order generation already
    // fixed (newest first), then id. `total_cmp` rather than `partial_cmp` so a
    // NaN could not silently reorder the ranking.
    scored.sort_by(|left, right| {
        right.confidence.total_cmp(&left.confidence).then_with(|| {
            left.candidate
                .candidate_id
                .cmp(&right.candidate.candidate_id)
        })
    });
    for (rank, candidate) in scored.iter_mut().enumerate() {
        candidate.rank = rank;
    }
    scored
}

/// Whether the top candidate is separated enough from the runner-up.
pub fn is_unambiguous(scored: &[ScoredCandidate]) -> bool {
    match scored {
        [] => false,
        [_] => true,
        [top, runner_up, ..] => top.confidence - runner_up.confidence >= AMBIGUITY_MARGIN,
    }
}
