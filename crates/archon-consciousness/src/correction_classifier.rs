//! Versioned, abstaining correction classifier (learning roadmap R3, Slice 1).
//!
//! Production correction detection is a phrase table: lowercase the turn, then
//! `starts_with("no,")`, `contains("i said")`, and so on. It has no notion of
//! how sure it is, so every match is treated as certain, and a match is what
//! reinforces a behavioural rule. The 2026-07-11 core audit records the cost at
//! its line 281 -- the heuristics "misfire constantly and every misfire becomes
//! a permanent rule" -- and that audit is a working document held outside the
//! repository, so the finding is quoted here rather than linked. A turn that
//! pastes a build log containing
//! "should have" is, to that table, indistinguishable from a user saying it.
//!
//! What the roadmap asks for is not a better phrase table. It is a classifier
//! that reports how sure it is and is permitted to say "I don't know": below
//! the threshold it abstains, and an abstention creates nothing -- no
//! correction, no lesson, no proposal, no rule mutation.
//!
//! Two arms, deliberately asymmetric:
//!
//! * The deterministic arm is the existing phrase table, moved here unchanged
//!   so `archon-core` and this crate cannot drift into two taxonomies. Its
//!   recall over those phrasings must stay exactly 1.0 -- the R3 promotion gate
//!   requires it -- so it is consulted first and nothing overrides its verdict.
//! * The provider arm exists for the language the table was never going to
//!   catch ("that's not what I meant"). It is OFF by default and needs BOTH a
//!   config flag and an injected provider, because a model call on every user
//!   turn is latency and cost the fast path has not agreed to pay.
//!
//! Nothing here mutates anything. Roadmap line 37 permits shadow measurement
//! while the R0 entry-gate items close but forbids mutating rules through this
//! classifier, and promotion needs 400 adjudicated examples (line 300). Callers
//! run this alongside the live heuristic and record what each one decided.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::corrections::CorrectionType;

/// Identity of the classifier's decision procedure.
///
/// Recorded on every shadow label so a window measured under one version is
/// never silently pooled with another. Changing an arm, a threshold, or the
/// phrase table is a version bump, not an edit to past rows.
pub const CORRECTION_CLASSIFIER_VERSION: &str = "correction-classifier/v1";

/// Confidence assigned to a deterministic phrase match.
///
/// Deliberately below 1.0: these phrasings are strong evidence, not proof --
/// the pasted-build-log case is exactly a phrase match that is not a
/// correction. High enough to clear any sane threshold, so the R3 requirement
/// that explicit-case recall stays 1.0 holds by construction.
pub const EXPLICIT_PHRASE_CONFIDENCE: f32 = 0.95;

/// Default confidence floor below which the classifier abstains.
///
/// Applies only to the provider arm; the deterministic arm sits well above it.
/// A provider that is merely leaning one way must produce an abstention, not a
/// rule.
pub const DEFAULT_ABSTAIN_BELOW: f32 = 0.60;

// ── rationale codes ──────────────────────────────────────────
//
// Stable, machine-readable, and versioned with the classifier. Every abstention
// code starts with `abstain.` so consumers can test for abstention without
// enumerating reasons they do not know about yet.

/// A deterministic phrase matched; the suffix names the matched taxonomy arm.
pub const RATIONALE_EXPLICIT_PHRASE_PREFIX: &str = "explicit_phrase.";
/// The provider arm returned a judgement at or above the threshold.
pub const RATIONALE_PROVIDER_JUDGED: &str = "provider.judged";
/// Prefix shared by every abstention rationale.
pub const RATIONALE_ABSTAIN_PREFIX: &str = "abstain.";
/// No phrase matched and the provider arm is not available to look further.
pub const RATIONALE_ABSTAIN_NO_SIGNAL: &str = "abstain.no_signal";
/// The provider arm answered, but under the confidence floor.
pub const RATIONALE_ABSTAIN_BELOW_THRESHOLD: &str = "abstain.below_threshold";
/// The provider arm is enabled but produced nothing usable this turn.
pub const RATIONALE_ABSTAIN_PROVIDER_UNAVAILABLE: &str = "abstain.provider_unavailable";

impl CorrectionType {
    /// Stable snake_case code for metric identities and rationale codes.
    ///
    /// Distinct from the `Debug` rendering used by the existing event payload:
    /// that one is a display detail and may change; this one is written into
    /// append-only measurement rows and may not.
    pub fn as_code(self) -> &'static str {
        match self {
            Self::FactualError => "factual_error",
            Self::ApproachCorrection => "approach_correction",
            Self::RepeatedInstruction => "repeated_instruction",
            Self::DidForbiddenAction => "did_forbidden_action",
            Self::ActedWithoutPermission => "acted_without_permission",
        }
    }
}

/// One classification decision, in the shape the roadmap specifies.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CorrectionClassification {
    pub is_correction: bool,
    pub correction_type: Option<CorrectionType>,
    pub confidence: f32,
    pub rationale_code: String,
}

impl CorrectionClassification {
    /// Whether this decision is an abstention.
    ///
    /// An abstention is NOT the same as a confident "not a correction": the
    /// first says the classifier declined to answer, the second is an answer.
    /// Conflating them would let abstentions into the precision denominator.
    pub fn abstained(&self) -> bool {
        self.rationale_code.starts_with(RATIONALE_ABSTAIN_PREFIX)
    }

    /// The label written to `predicted_label` on a metric event.
    pub fn predicted_label(&self) -> &'static str {
        if self.abstained() {
            "abstain"
        } else if self.is_correction {
            "correction"
        } else {
            "not_correction"
        }
    }

    fn abstain(rationale_code: &str, confidence: f32) -> Self {
        Self {
            // An abstention asserts nothing, so it carries the same fields a
            // negative answer would; `abstained()` is what tells them apart.
            is_correction: false,
            correction_type: None,
            confidence,
            rationale_code: rationale_code.to_string(),
        }
    }
}

/// A judgement from the provider arm, before thresholding.
#[derive(Debug, Clone, PartialEq)]
pub struct ProviderJudgement {
    pub is_correction: bool,
    pub correction_type: Option<CorrectionType>,
    pub confidence: f32,
}

/// The ambiguous-language arm.
///
/// A trait rather than a concrete client so this crate keeps zero dependency on
/// any model transport, and so tests can drive the arm deterministically. The
/// implementation lives wherever the caller can afford a model call.
///
/// Returning `None` means "no usable answer" and produces an abstention. An
/// implementation must never guess to fill the gap.
pub trait AmbiguousCorrectionProvider: Send + Sync {
    fn judge(&self, user_input: &str) -> Option<ProviderJudgement>;
}

/// Configuration for one classifier version.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CorrectionClassifierConfig {
    /// Identity recorded on every decision made under this configuration.
    pub version: String,
    /// Confidence floor for the provider arm.
    pub abstain_below: f32,
    /// Whether the provider arm may run at all.
    ///
    /// Off by default. Turning it on is not enough on its own -- a provider
    /// must also be injected -- so a stray `true` in a config file cannot make
    /// the fast path start calling a model that was never wired.
    pub provider_enabled: bool,
}

impl Default for CorrectionClassifierConfig {
    fn default() -> Self {
        Self {
            version: CORRECTION_CLASSIFIER_VERSION.to_string(),
            abstain_below: DEFAULT_ABSTAIN_BELOW,
            provider_enabled: false,
        }
    }
}

/// The classifier itself.
#[derive(Clone, Default)]
pub struct CorrectionClassifier {
    config: CorrectionClassifierConfig,
    provider: Option<Arc<dyn AmbiguousCorrectionProvider>>,
}

impl std::fmt::Debug for CorrectionClassifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CorrectionClassifier")
            .field("config", &self.config)
            .field("provider_injected", &self.provider.is_some())
            .finish()
    }
}

impl CorrectionClassifier {
    pub fn new(config: CorrectionClassifierConfig) -> Self {
        Self {
            config,
            provider: None,
        }
    }

    pub fn with_provider(mut self, provider: Arc<dyn AmbiguousCorrectionProvider>) -> Self {
        self.provider = Some(provider);
        self
    }

    pub fn config(&self) -> &CorrectionClassifierConfig {
        &self.config
    }

    pub fn version(&self) -> &str {
        &self.config.version
    }

    /// Classify one user turn.
    ///
    /// Order matters: the deterministic arm answers first and is never
    /// second-guessed, so its recall cannot regress when the provider arm is
    /// later switched on.
    pub fn classify(&self, user_input: &str) -> CorrectionClassification {
        if let Some(correction_type) = explicit_phrase_match(user_input) {
            return CorrectionClassification {
                is_correction: true,
                correction_type: Some(correction_type),
                confidence: EXPLICIT_PHRASE_CONFIDENCE,
                rationale_code: format!(
                    "{RATIONALE_EXPLICIT_PHRASE_PREFIX}{}",
                    correction_type.as_code()
                ),
            };
        }

        if !self.config.provider_enabled {
            // The common case today, and the honest one: with no second arm
            // there is no evidence either way, so the classifier declines
            // rather than reporting a confident "not a correction" it cannot
            // support.
            return CorrectionClassification::abstain(RATIONALE_ABSTAIN_NO_SIGNAL, 0.0);
        }

        let Some(judgement) = self
            .provider
            .as_ref()
            .and_then(|provider| provider.judge(user_input))
        else {
            // Enabled but unusable -- missing injection, or the provider
            // declined. Telemetry may fail open; a classification may not.
            return CorrectionClassification::abstain(RATIONALE_ABSTAIN_PROVIDER_UNAVAILABLE, 0.0);
        };

        // A non-finite or out-of-range confidence would poison every downstream
        // mean, threshold, and Brier score, so it is treated as no answer
        // rather than clamped into a number the provider never gave.
        if !judgement.confidence.is_finite() || !(0.0..=1.0).contains(&judgement.confidence) {
            return CorrectionClassification::abstain(RATIONALE_ABSTAIN_PROVIDER_UNAVAILABLE, 0.0);
        }
        if judgement.confidence < self.config.abstain_below {
            return CorrectionClassification::abstain(
                RATIONALE_ABSTAIN_BELOW_THRESHOLD,
                judgement.confidence,
            );
        }

        CorrectionClassification {
            is_correction: judgement.is_correction,
            // A positive judgement with no taxonomy is still a positive
            // judgement; the type stays `None` rather than being invented,
            // because the fallback that invents one is how every unlabelled
            // correction became an `ApproachCorrection`.
            correction_type: judgement
                .is_correction
                .then_some(judgement.correction_type)
                .flatten(),
            confidence: judgement.confidence,
            rationale_code: RATIONALE_PROVIDER_JUDGED.to_string(),
        }
    }
}

/// The deterministic phrase table.
///
/// Moved here from `archon-core::agent::correction_intake` so there is one
/// table rather than two that drift. `None` means "no phrase matched", which is
/// a statement about this table and not about whether the user corrected
/// anything -- the classifier's abstention and the periodic semantic pass are
/// what decide that.
pub fn explicit_phrase_match(user_input: &str) -> Option<CorrectionType> {
    let lower = user_input.to_lowercase();
    if lower.starts_with("no,")
        || lower.starts_with("no ")
        || lower.starts_with("wrong")
        || lower.starts_with("that's wrong")
        || lower.starts_with("that is wrong")
    {
        Some(CorrectionType::FactualError)
    } else if lower.contains("i said")
        || lower.contains("i already told you")
        || lower.contains("i already asked")
        || lower.contains("as i mentioned")
    {
        Some(CorrectionType::RepeatedInstruction)
    } else if lower.starts_with("don't ")
        || lower.starts_with("do not ")
        || lower.starts_with("stop ")
        || lower.contains("never do that")
    {
        Some(CorrectionType::DidForbiddenAction)
    } else if lower.contains("didn't ask")
        || lower.contains("did not ask")
        || lower.contains("without permission")
        || lower.contains("without asking")
    {
        Some(CorrectionType::ActedWithoutPermission)
    } else if lower.contains("instead,")
        || lower.contains("should have")
        || lower.contains("better approach")
        || lower.contains("use this instead")
    {
        Some(CorrectionType::ApproachCorrection)
    } else {
        None
    }
}

// ── tests ────────────────────────────────────────────────────

#[cfg(test)]
#[path = "correction_classifier/tests.rs"]
mod tests;
