//! Where a correction came from, read out of the correction RECORD.
//!
//! R0 finding 41 closed by making the correction-derived rule identity a
//! compile-time constant per [`CorrectionType`]: five rules serve every
//! correction this system will ever record. `docs/development/r0-entry-gate.evidence`
//! states the consequence for this slice in as many words -- "R2/R3 lesson
//! provenance must therefore come from the correction records, not from rule
//! text". A rule id now names a CATEGORY. It cannot say which turn went wrong,
//! which action caused it, or which lesson it stands for, because five strings
//! cannot carry that and never did.
//!
//! So provenance is read from the correction instead, and the only structured
//! field a [`Correction`] has for it is `context`. That field is written by the
//! agent as `turn:{n}` and by the deferred semantic pass as
//! `turn:{n} (semantic pass)`. This module owns both halves of that encoding --
//! the writer and the parser -- so the format has one definition rather than a
//! `format!` at one end and a `strip_prefix` at the other.
//!
//! What the record does NOT carry is session identity. There is no session
//! field on [`Correction`] and `context` has never held one, so a correction
//! read back out of the graph cannot say which session produced it. That is
//! load-bearing for attribution rather than a cosmetic gap: attributing a
//! correction to an action means naming a specific tool run in a specific
//! session, and a provenance record that cannot name the session cannot support
//! that claim. [`CorrectionProvenance`] therefore reports what the record has
//! and no more; the caller supplies session identity from the live turn, and the
//! attribution engine refuses candidates from any other session.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::corrections::{Correction, CorrectionType};

/// Prefix of the `context` field written for every correction.
pub const CORRECTION_CONTEXT_TURN_PREFIX: &str = "turn:";

/// Suffix marking a correction found by the deferred semantic pass.
pub const SEMANTIC_PASS_MARKER: &str = " (semantic pass)";

/// Which detector wrote the correction.
///
/// Not decoration: the immediate pass records during the turn it observed, so
/// the actions it may be attributed to are still in the conversation the agent
/// is holding. The semantic pass records from a background task several turns
/// later, against a window that has already moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CorrectionPass {
    /// The keyword detector, during the turn it observed.
    Immediate,
    /// The periodic extractor, after the fact.
    SemanticExtraction,
    /// The context did not parse. Not "probably immediate" -- unknown.
    Unrecognised,
}

impl CorrectionPass {
    /// Stable snake_case code for measurement rows.
    pub fn as_code(self) -> &'static str {
        match self {
            Self::Immediate => "immediate",
            Self::SemanticExtraction => "semantic_extraction",
            Self::Unrecognised => "unrecognised",
        }
    }
}

/// The `context` value for a correction the keyword detector caught in turn
/// `turn_number`.
pub fn immediate_turn_context(turn_number: u64) -> String {
    format!("{CORRECTION_CONTEXT_TURN_PREFIX}{turn_number}")
}

/// The `context` value for a correction the semantic pass found afterwards.
pub fn semantic_pass_context(turn_number: u64) -> String {
    format!("{CORRECTION_CONTEXT_TURN_PREFIX}{turn_number}{SEMANTIC_PASS_MARKER}")
}

/// Read a `context` value back.
///
/// Returns `None` for the turn when the string is not one this module wrote.
/// Deliberately not a fallback to turn zero or to "the current turn": a
/// correction whose turn is unknown must be recorded as unattributable, and a
/// guessed turn number would instead attribute it to whatever happened to be in
/// the window.
pub fn parse_correction_context(context: &str) -> (Option<u64>, CorrectionPass) {
    let (body, pass) = match context.strip_suffix(SEMANTIC_PASS_MARKER) {
        Some(body) => (body, CorrectionPass::SemanticExtraction),
        None => (context, CorrectionPass::Immediate),
    };
    match body
        .strip_prefix(CORRECTION_CONTEXT_TURN_PREFIX)
        .map(str::trim)
        .and_then(|turn| turn.parse::<u64>().ok())
    {
        Some(turn_number) => (Some(turn_number), pass),
        None => (None, CorrectionPass::Unrecognised),
    }
}

/// Reason a provenance record cannot support an attribution.
pub const PROVENANCE_UNPARSED_TURN: &str = "provenance.unparsed_turn";
/// Turn numbers start at 1; a zero means the writer never set one.
pub const PROVENANCE_ZERO_TURN: &str = "provenance.zero_turn";

/// What a stored correction can say about its own origin.
#[derive(Debug, Clone, PartialEq)]
pub struct CorrectionProvenance {
    pub correction_id: String,
    pub correction_type: CorrectionType,
    /// The turn the correction was recorded against, when the context parsed.
    pub turn_number: Option<u64>,
    pub pass: CorrectionPass,
    pub severity: f64,
    pub recorded_at: DateTime<Utc>,
    /// The category rule the correction was linked to, when one was linked.
    ///
    /// Recorded for a human reading a measurement row, and excluded from
    /// [`Self::evidence_refs`] on purpose -- see the note there.
    pub linked_rule_id: Option<String>,
}

impl CorrectionProvenance {
    /// Read provenance out of a stored correction.
    pub fn from_record(correction: &Correction) -> Self {
        let (turn_number, pass) = parse_correction_context(&correction.context);
        Self {
            correction_id: correction.id.clone(),
            correction_type: correction.correction_type,
            turn_number,
            pass,
            severity: correction.severity,
            recorded_at: correction.timestamp,
            linked_rule_id: correction.rule_id.clone(),
        }
    }

    /// Why this provenance cannot anchor an attribution, if it cannot.
    ///
    /// `None` means the record names a turn, which is the minimum an attribution
    /// needs before the caller adds session identity.
    pub fn incompleteness_code(&self) -> Option<&'static str> {
        match self.turn_number {
            None => Some(PROVENANCE_UNPARSED_TURN),
            Some(0) => Some(PROVENANCE_ZERO_TURN),
            Some(_) => None,
        }
    }

    /// Typed join keys for anything derived from this correction.
    ///
    /// The linked rule id is NOT among them. Since finding 41 the rule is one of
    /// five constants shared by every correction of its type, so a lesson that
    /// cited it as its source would be citing "corrections of this kind exist" --
    /// true of thousands of unrelated turns, and useless for deciding whether a
    /// later failure is a repeat of this one. The correction id and turn are the
    /// refs that discriminate.
    pub fn evidence_refs(&self) -> Vec<String> {
        let mut refs = vec![
            format!("correction:{}", self.correction_id),
            format!("correction_type:{}", self.correction_type.as_code()),
            format!("correction_pass:{}", self.pass.as_code()),
        ];
        if let Some(turn_number) = self.turn_number {
            refs.push(format!("turn:{turn_number}"));
        }
        refs
    }
}

#[cfg(test)]
#[path = "correction_provenance/tests.rs"]
mod tests;
