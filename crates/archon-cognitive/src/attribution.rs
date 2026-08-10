//! R2 causal credit assignment: which decision or action caused this
//! correction.
//!
//! `MetricEventKind::AttributionEvaluated` and its mandatory identity keys have
//! existed since the R8 measurement schema landed, with nothing writing one.
//! This module is what writes the first: it takes a correction and the actions
//! that preceded it, ranks the things that could have caused it, and produces an
//! adjudicable claim.
//!
//! Three properties are load-bearing, and each of them is a thing this codebase
//! got wrong before.
//!
//! **An attribution is a claim, not a fact.** Every assessment carries a
//! confidence and the typed evidence it rests on, and the adjudicated candidate
//! id it is scored against is `pending_adjudication:*` until a human supplies
//! one. The roadmap defines an accepted link as correct only when
//! `causal_candidate_id == adjudicated_causal_candidate_id`; the pending
//! sentinel can never equal a proposed candidate id, so precision computed today
//! reads as zero-correct rather than as a fabricated 1.0. That is the intended
//! behaviour, not an oversight -- see [`event::ADJUDICATION_PENDING_PREFIX`].
//!
//! **An unattributable correction is recorded as unattributed.** There is no
//! path that assigns the nearest candidate to avoid an empty result.
//! [`AttributionAssessment::accepted_candidate`] returns `None` for every
//! outcome except an acceptance, even though the ranked list is still recorded
//! as evidence, so a caller cannot read a cause out of an abstention by
//! accident.
//!
//! **Shadow only.** The R0 entry gate forbids rule mutation through new learning
//! classifiers before promotion. That is enforced structurally rather than by a
//! default someone can flip:
//!
//! * [`AttributionEngine::attribute`] takes owned plain data and returns a
//!   verdict. No store handle, memory graph, or rules engine is in scope while
//!   an attribution is decided, so there is nothing for it to mutate.
//! * `archon-cognitive` does not depend on `archon-consciousness` and cannot
//!   name `RulesEngine`, `CorrectionTracker`, or any rule type. Making this
//!   module able to mutate a rule requires a manifest change.
//! * [`ATTRIBUTION_MODE`] is a compile-time constant with no serde derive, no
//!   config field, and no environment read. Every emitted event records it, so
//!   "nothing was mutated" is checkable from the rows rather than asserted in a
//!   comment.
//!
//! Promotion needs 200 adjudicated attributions with at least 100 accepted links
//! and at least 100 eligible repeated opportunities per cohort
//! (`docs/development/learning-roadmap-r1-r8-w5-w6.md`, promotion-gate table).
//! None of that corpus exists. This produces the rows it would be built from.

use crate::attribution::candidates::generate;
use crate::attribution::input::AttributionInput;
use crate::attribution::scoring::{ACCEPT_CONFIDENCE, ScoredCandidate, is_unambiguous, score};

#[path = "attribution/candidates.rs"]
pub mod candidates;
#[path = "attribution/event.rs"]
pub mod event;
#[path = "attribution/input.rs"]
pub mod input;
#[path = "attribution/scoring.rs"]
pub mod scoring;

/// Identity of this attribution procedure.
///
/// Recorded on every row. Changing a weight, a threshold, or an evidence kind is
/// a version bump, so a window measured under one procedure is never pooled with
/// another.
pub const CAUSAL_ATTRIBUTION_VERSION: &str = "causal-attribution/v1";

/// Whether attribution may influence anything.
///
/// Deliberately not a config field. A `provider_enabled: bool` in a struct is
/// one stray `true` in a TOML file away from being on; a constant is a code
/// change that appears in a diff and trips the R0 entry gate's shadow
/// containment check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttributionMode {
    /// Measure and record. Nothing downstream reads the verdict.
    Shadow,
    /// Reserved for the state after the R2 promotion gate passes. Nothing
    /// constructs this today.
    Promoted,
}

impl AttributionMode {
    pub fn as_code(self) -> &'static str {
        match self {
            Self::Shadow => "shadow",
            Self::Promoted => "promoted",
        }
    }
}

/// The mode in force. Shadow until the R2 promotion gate passes.
pub const ATTRIBUTION_MODE: AttributionMode = AttributionMode::Shadow;

/// What kind of thing a correction was attributed to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CauseActionClass {
    /// `Correction -> CausedBy -> ToolRun`.
    ToolRun,
    /// `Correction -> Corrects -> Decision`.
    Decision,
}

impl CauseActionClass {
    pub fn as_code(self) -> &'static str {
        match self {
            Self::ToolRun => "tool_run",
            Self::Decision => "decision",
        }
    }
}

/// `cause_action_class` for an outcome that names no cause.
pub const CAUSE_ACTION_CLASS_NONE: &str = "none";

/// Which of the three follow-up cohorts this correction joins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttributionCohort {
    /// A causal link was proposed.
    Accepted,
    /// Candidates existed; none was supportable.
    Abstained,
    /// There was nothing to choose between, or the correction could not be
    /// placed in the conversation at all.
    Unattributed,
}

impl AttributionCohort {
    pub fn as_code(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Abstained => "abstained",
            Self::Unattributed => "unattributed",
        }
    }
}

// ── rationale codes ──────────────────────────────────────────
//
// Same shape as the R3 classifier's: stable, prefix-grouped, versioned with the
// procedure. A consumer tests the prefix rather than enumerating reasons it does
// not know about yet.

/// The top candidate carries deterministic evidence of its own failure.
pub const RATIONALE_ATTRIBUTED_DETERMINISTIC: &str = "attributed.deterministic";
/// The top candidate is supported by structural, non-lexical evidence.
pub const RATIONALE_ATTRIBUTED_CORROBORATED: &str = "attributed.corroborated";
/// Prefix shared by every abstention.
pub const RATIONALE_ABSTAIN_PREFIX: &str = "abstain.";
/// The best candidate did not reach the confidence floor.
pub const RATIONALE_ABSTAIN_BELOW_THRESHOLD: &str = "abstain.below_threshold";
/// Two candidates are too close to separate.
pub const RATIONALE_ABSTAIN_AMBIGUOUS: &str = "abstain.ambiguous";
/// Only recency and word overlap support the top candidate.
pub const RATIONALE_ABSTAIN_UNCORROBORATED: &str = "abstain.uncorroborated";
/// Prefix shared by every unattributed outcome.
pub const RATIONALE_UNATTRIBUTED_PREFIX: &str = "unattributed.";

/// One attribution decision.
#[derive(Debug, Clone, PartialEq)]
pub struct AttributionAssessment {
    pub attributed: bool,
    pub cohort: AttributionCohort,
    pub confidence: f32,
    pub rationale_code: String,
    /// Every candidate that was considered, best first. Recorded even when the
    /// outcome is an abstention, because "these five were considered and none
    /// won" is what makes the abstention reviewable.
    pub ranked: Vec<ScoredCandidate>,
    pub version: &'static str,
}

impl AttributionAssessment {
    pub fn abstained(&self) -> bool {
        self.rationale_code.starts_with(RATIONALE_ABSTAIN_PREFIX)
    }

    pub fn unattributed(&self) -> bool {
        self.rationale_code
            .starts_with(RATIONALE_UNATTRIBUTED_PREFIX)
    }

    /// The candidate this correction is attributed to, if any.
    ///
    /// `None` unless the outcome is an acceptance, regardless of what
    /// [`Self::ranked`] contains. This is the "never assign the nearest
    /// candidate" rule expressed where a caller would otherwise be tempted:
    /// reading `ranked[0]` after an abstention is reading a candidate that was
    /// explicitly rejected.
    pub fn accepted_candidate(&self) -> Option<&ScoredCandidate> {
        if !self.attributed {
            return None;
        }
        self.ranked.first()
    }

    pub fn cause_action_class_code(&self) -> &'static str {
        self.accepted_candidate()
            .map_or(CAUSE_ACTION_CLASS_NONE, |scored| {
                scored.candidate.cause_action_class.as_code()
            })
    }

    fn refused(
        cohort: AttributionCohort,
        rationale_code: &str,
        ranked: Vec<ScoredCandidate>,
    ) -> Self {
        let confidence = ranked.first().map_or(0.0, |scored| scored.confidence);
        Self {
            // A refusal asserts no cause. It still carries the confidence of the
            // best candidate, because "we abstained at 0.52" and "we abstained
            // with nothing above 0.1" are different facts about the corpus.
            attributed: false,
            cohort,
            confidence,
            rationale_code: rationale_code.to_string(),
            ranked,
            version: CAUSAL_ATTRIBUTION_VERSION,
        }
    }
}

/// The attribution procedure.
///
/// A unit struct on purpose: there is no configuration, because every knob would
/// be a way to loosen the acceptance rule without a version bump.
#[derive(Debug, Clone, Copy, Default)]
pub struct AttributionEngine;

impl AttributionEngine {
    pub fn version(&self) -> &'static str {
        CAUSAL_ATTRIBUTION_VERSION
    }

    pub fn mode(&self) -> AttributionMode {
        ATTRIBUTION_MODE
    }

    /// Attribute one correction.
    ///
    /// Pure: owned data in, verdict out. See the module note on why the
    /// signature is the containment.
    pub fn attribute(&self, input: &AttributionInput) -> AttributionAssessment {
        if let Some(rationale) = input.precondition_failure() {
            return AttributionAssessment::refused(
                AttributionCohort::Unattributed,
                rationale,
                Vec::new(),
            );
        }

        let candidates = generate(input);
        if candidates.is_empty() {
            return AttributionAssessment::refused(
                AttributionCohort::Unattributed,
                input::UNATTRIBUTED_NO_ELIGIBLE_CANDIDATE,
                Vec::new(),
            );
        }

        let ranked = score(input, candidates);
        let Some(top) = ranked.first() else {
            return AttributionAssessment::refused(
                AttributionCohort::Unattributed,
                input::UNATTRIBUTED_NO_ELIGIBLE_CANDIDATE,
                ranked,
            );
        };

        // Order matters: the corroboration rule is checked before the threshold
        // so a candidate that scored well purely on recency and word overlap
        // reports WHY it was refused rather than looking like a near miss.
        if !top.has_corroboration() {
            return AttributionAssessment::refused(
                AttributionCohort::Abstained,
                RATIONALE_ABSTAIN_UNCORROBORATED,
                ranked,
            );
        }
        if top.confidence < ACCEPT_CONFIDENCE {
            return AttributionAssessment::refused(
                AttributionCohort::Abstained,
                RATIONALE_ABSTAIN_BELOW_THRESHOLD,
                ranked,
            );
        }
        if !is_unambiguous(&ranked) {
            return AttributionAssessment::refused(
                AttributionCohort::Abstained,
                RATIONALE_ABSTAIN_AMBIGUOUS,
                ranked,
            );
        }

        let deterministic = top.evidence.iter().any(|evidence| {
            matches!(
                evidence.kind,
                scoring::EvidenceKind::DeterministicFailure
                    | scoring::EvidenceKind::PermissionBlock
            )
        });
        let confidence = top.confidence;
        AttributionAssessment {
            attributed: true,
            cohort: AttributionCohort::Accepted,
            confidence,
            rationale_code: if deterministic {
                RATIONALE_ATTRIBUTED_DETERMINISTIC.to_string()
            } else {
                RATIONALE_ATTRIBUTED_CORROBORATED.to_string()
            },
            ranked,
            version: CAUSAL_ATTRIBUTION_VERSION,
        }
    }
}

#[cfg(test)]
#[path = "attribution/tests.rs"]
mod tests;
