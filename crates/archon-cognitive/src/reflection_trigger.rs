//! When a turn earns a reflection.
//!
//! Reflection used to be reachable only from inside `ExecutiveLoop::run_turn`,
//! which nothing called, so in practice nothing ever reflected. The triggers
//! here are the ones issue #81 names, expressed over signals a live turn
//! actually produces, and they are pure functions so the decision to reflect is
//! testable without a database, a model, or a turn.
//!
//! Every trigger is bounded on purpose: an unbounded one would fire on most
//! turns and turn the reflection relation into a turn log.

use serde::{Deserialize, Serialize};

use crate::SituationKind;

/// Surprise at or above which a turn is worth reflecting on.
///
/// Set where a plain action-class mismatch alone (0.5) does *not* trigger:
/// disagreeing with the shadow planner is ordinary. It takes a mismatch plus
/// something else going wrong.
pub const HIGH_SURPRISE_THRESHOLD: f32 = 0.6;

/// Failing tool calls in one turn that count as "repeated".
///
/// Two, not one: a single failed call is normal exploration, and reflecting on
/// each would drown the genuinely repeated failures the trigger exists for.
pub const REPEATED_TOOL_FAILURE_THRESHOLD: u32 = 2;

/// Classifier confidence at or above which a correction counts as
/// high-confidence.
pub const HIGH_CONFIDENCE_CORRECTION_MIN: f32 = 0.8;

/// Failure count at which the repeated-failure confidence saturates.
const TOOL_FAILURE_CONFIDENCE_CEILING: f32 = 4.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReflectionTrigger {
    HighConfidenceCorrection,
    RepeatedToolFailure,
    HighSurprise,
}

impl ReflectionTrigger {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::HighConfidenceCorrection => "high_confidence_correction",
            Self::RepeatedToolFailure => "repeated_tool_failure",
            Self::HighSurprise => "high_surprise",
        }
    }
}

/// Signals one finished turn produced.
///
/// Only counts, enums and bounded scalars: there is no field a caller could put
/// raw model reasoning or user text into, which is what keeps
/// `ReflectionWriter` structurally unable to persist chain-of-thought.
#[derive(Debug, Clone, PartialEq)]
pub struct TurnSignals {
    pub situation_kind: SituationKind,
    /// Shadow-vs-live surprise in `[0, 1]`, or `None` when the turn had no
    /// shadow plan to compare against.
    pub shadow_surprise: Option<f32>,
    pub tool_failures: u32,
    /// Confidence of the correction detector, or `None` when no correction was
    /// detected on this turn.
    pub correction_confidence: Option<f32>,
    pub completed: bool,
}

impl TurnSignals {
    pub fn new(situation_kind: SituationKind) -> Self {
        Self {
            situation_kind,
            shadow_surprise: None,
            tool_failures: 0,
            correction_confidence: None,
            completed: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TriggeredReflection {
    pub trigger: ReflectionTrigger,
    /// How strongly the signal fired, in `[0, 1]`. Persisted with the
    /// reflection so a later reader can weight it instead of treating every
    /// reflection as equally certain.
    pub confidence: f32,
}

/// Decide whether `signals` warrant a reflection, and how confident it is.
///
/// At most one reflection per turn, by construction: the triggers are ordered
/// by how directly they carry information about the agent's own behaviour, and
/// the first match wins. A user correction is the strongest of the three — it
/// is the only one where a human told us we were wrong.
pub fn evaluate(signals: &TurnSignals) -> Option<TriggeredReflection> {
    if let Some(confidence) = bounded(signals.correction_confidence)
        && confidence >= HIGH_CONFIDENCE_CORRECTION_MIN
    {
        return Some(TriggeredReflection {
            trigger: ReflectionTrigger::HighConfidenceCorrection,
            confidence,
        });
    }
    if signals.tool_failures >= REPEATED_TOOL_FAILURE_THRESHOLD {
        return Some(TriggeredReflection {
            trigger: ReflectionTrigger::RepeatedToolFailure,
            confidence: (signals.tool_failures as f32 / TOOL_FAILURE_CONFIDENCE_CEILING)
                .clamp(0.5, 1.0),
        });
    }
    if let Some(surprise) = bounded(signals.shadow_surprise)
        && surprise >= HIGH_SURPRISE_THRESHOLD
    {
        return Some(TriggeredReflection {
            trigger: ReflectionTrigger::HighSurprise,
            confidence: surprise,
        });
    }
    None
}

/// Reject a score that is not a probability.
///
/// A NaN comparison is false in every direction, so an unchecked NaN would
/// silently disable a trigger rather than announce itself; an out-of-range
/// value would be a caller bug that clamping would hide.
fn bounded(value: Option<f32>) -> Option<f32> {
    value.filter(|value| value.is_finite() && (0.0..=1.0).contains(value))
}
