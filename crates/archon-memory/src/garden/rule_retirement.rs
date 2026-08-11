//! Rule-retirement proposals: which prompt rules have stopped earning a slot.
//!
//! The prompt carries a bounded block of behavioural rules. A rule that no
//! longer corresponds to anything the user actually corrects keeps occupying a
//! slot a live rule could use, and keeps steering the model toward a problem
//! that stopped happening. Noticing that is what this module does.
//!
//! # It cannot mutate a rule, structurally
//!
//! [`rule_retirement_candidates`] takes `&[RuleObservation]` — plain read-only
//! data — and returns values. It has no store handle, no rules engine, and no
//! `&dyn MemoryTrait`. There is nothing in scope for it to write through, so
//! "generation does not mutate rules" is a property of the signature rather than
//! a discipline the body has to keep. This is the same guarantee, by the same
//! means, as the scheduled pass's hard-coded zero deletion allowance.
//!
//! # Provenance comes from correction records, not rule text
//!
//! Rules ship as a small set of fixed identities, one per correction category,
//! with constant text. That was the right fix for an earlier defect — raw user
//! text can never reach a rule body — but it means rule text carries no lesson
//! provenance at all: every rule of a given category reads identically whether
//! it was derived from one correction or a hundred.
//!
//! So the evidence for retiring a rule is the CORRECTIONS that support it: how
//! many there are and when the most recent one was recorded. A rule whose
//! supporting corrections have gone quiet is a rule about a problem that stopped
//! happening. [`RuleObservation`] carries those counts because the caller reads
//! them from correction records; nothing here parses rule text for meaning.
//!
//! # A rule the user wrote is never proposed
//!
//! Only rules a correction derived are candidates. A rule someone typed is a
//! statement of intent, and its going quiet means the model stopped needing to
//! be told — which is the rule working, not the rule expiring.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

/// Where a rule came from, as far as retirement is concerned.
///
/// Mirrors the rules engine's own source classification, restated here as plain
/// data so this crate needs no dependency on the crate that owns rules — which
/// is also what keeps a rules engine out of reach of this analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuleOrigin {
    /// Someone typed it. Never proposed for retirement.
    UserDefined,
    /// Derived from a correction category.
    CorrectionDerived,
    /// Shipped as a default. Never proposed: it is operator policy, not learned
    /// evidence, and removing it would be a config change made by a background
    /// job.
    SystemDefault,
}

/// A read-only snapshot of one rule and the corrections behind it.
///
/// Every field is data the caller has already read. Nothing here can be written
/// back through this type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleObservation {
    pub rule_id: String,
    /// For display in the proposal only. Never parsed for meaning.
    pub rule_text: String,
    pub score: f64,
    pub origin: RuleOrigin,
    pub created_at: DateTime<Utc>,
    /// When the rule was last reinforced by a correction matching it.
    pub last_triggered: Option<DateTime<Utc>>,
    /// How many correction records currently name this rule.
    pub supporting_corrections: usize,
    /// When the most recent supporting correction was recorded.
    pub most_recent_correction: Option<DateTime<Utc>>,
    /// Whether the rule is currently inside the prompt block.
    ///
    /// A rule already below the score floor is not occupying a slot, so
    /// retiring it would change nothing a user could observe while still
    /// spending a review decision.
    pub in_prompt: bool,
}

/// When a rule stops earning its slot.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RuleRetirementPolicy {
    /// How long a rule's supporting corrections must have been silent.
    pub quiet_days: i64,
    /// A rule younger than this is never proposed, however quiet.
    ///
    /// A rule created last week has not had time to be corroborated, and
    /// retiring it would mean the system forgetting a lesson before it had a
    /// chance to recur.
    pub min_age_days: i64,
    /// Below this many supporting corrections, a quiet rule is weakly held.
    pub max_supporting_corrections: usize,
}

impl Default for RuleRetirementPolicy {
    fn default() -> Self {
        Self {
            // Long enough that a quiet fortnight during a holiday does not
            // retire a rule the user still needs.
            quiet_days: 90,
            min_age_days: 30,
            // A rule with more supporting corrections than this has been earned
            // repeatedly; silence is more likely to mean it is working.
            max_supporting_corrections: 3,
        }
    }
}

/// Why a rule was proposed for retirement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleRetirementEvidence {
    pub days_since_supporting_correction: Option<i64>,
    pub days_since_triggered: Option<i64>,
    pub supporting_corrections: usize,
    pub score: f64,
    pub quiet_days: i64,
}

/// A prompt rule proposed for retirement, with the correction evidence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleRetirementCandidate {
    pub rule_id: String,
    pub rule_text: String,
    pub evidence: RuleRetirementEvidence,
}

/// Which observed rules have stopped earning their prompt slot.
///
/// Pure. `now` is a parameter rather than read from the clock so the policy is
/// testable at a fixed instant and two callers in one pass agree on "today".
pub fn rule_retirement_candidates(
    observations: &[RuleObservation],
    policy: &RuleRetirementPolicy,
    now: DateTime<Utc>,
) -> Vec<RuleRetirementCandidate> {
    observations
        .iter()
        .filter_map(|observation| evaluate(observation, policy, now))
        .collect()
}

fn evaluate(
    observation: &RuleObservation,
    policy: &RuleRetirementPolicy,
    now: DateTime<Utc>,
) -> Option<RuleRetirementCandidate> {
    // A rule someone typed, or one shipped as policy, is not this job's to
    // question. Checked first so no later condition can reach them.
    if observation.origin != RuleOrigin::CorrectionDerived {
        return None;
    }
    // Retiring a rule nobody sees spends a review decision on nothing.
    if !observation.in_prompt {
        return None;
    }
    if now - observation.created_at < Duration::days(policy.min_age_days) {
        return None;
    }
    if observation.supporting_corrections > policy.max_supporting_corrections {
        return None;
    }

    let quiet = Duration::days(policy.quiet_days);
    // Silence is only evidence if we know when the last signal was. An absent
    // timestamp means "never recorded", which for a rule older than
    // `min_age_days` is the strongest form of quiet, not an unknown.
    let correction_quiet = match observation.most_recent_correction {
        Some(at) => now - at >= quiet,
        None => true,
    };
    let trigger_quiet = match observation.last_triggered {
        Some(at) => now - at >= quiet,
        None => true,
    };
    // BOTH, not either. A rule with no recent correction but recent triggering
    // is still doing something; a rule that is triggering without corrections
    // behind it is a matching artefact worth keeping an eye on rather than
    // retiring silently.
    if !(correction_quiet && trigger_quiet) {
        return None;
    }

    Some(RuleRetirementCandidate {
        rule_id: observation.rule_id.clone(),
        rule_text: observation.rule_text.clone(),
        evidence: RuleRetirementEvidence {
            days_since_supporting_correction: observation
                .most_recent_correction
                .map(|at| (now - at).num_days()),
            days_since_triggered: observation.last_triggered.map(|at| (now - at).num_days()),
            supporting_corrections: observation.supporting_corrections,
            score: observation.score,
            quiet_days: policy.quiet_days,
        },
    })
}

#[cfg(test)]
#[path = "rule_retirement_tests.rs"]
mod rule_retirement_tests;
