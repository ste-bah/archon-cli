//! What the attribution engine is allowed to look at.
//!
//! Every type here is owned plain data. That is the containment: the engine's
//! entry point takes `&AttributionInput` and returns a verdict, so no store
//! handle, no memory graph, and no rules engine is ever in scope while an
//! attribution is being decided. A function that cannot name a mutable thing
//! cannot mutate one, which is a stronger statement than a configuration flag
//! that defaults to off.
//!
//! Assembling this is the caller's job, and it is where the I/O lives: reading
//! the conversation for tool runs, and the decision ledger for decisions.

use chrono::{DateTime, Utc};

/// Whether an action could change anything outside the process.
///
/// Supplied by the caller from the tool's own declared permission level rather
/// than inferred from its name here, so the classification tracks the tool
/// registry instead of a list in this crate that would drift from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionEffectClass {
    /// Observed the world without changing it.
    Read,
    /// Could change the world.
    Mutate,
    /// The tool was not resolvable when the window was assembled.
    Unknown,
}

impl ActionEffectClass {
    pub fn as_code(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Mutate => "mutate",
            Self::Unknown => "unknown",
        }
    }
}

/// The correction being explained.
#[derive(Debug, Clone, PartialEq)]
pub struct CorrectionUnderReview {
    pub correction_id: String,
    /// Supplied by the caller from the live turn. The correction record has no
    /// session field, so this cannot be read back out of the graph -- see
    /// `archon_consciousness::correction_provenance`.
    pub session_id: String,
    /// The turn the correction was recorded against, from its own provenance.
    pub turn_number: u64,
    /// `CorrectionType::as_code` of the recorded type.
    pub correction_type_code: String,
    /// The stored (already bounded) correction text.
    pub summary: String,
    pub recorded_at: DateTime<Utc>,
}

/// One tool execution observed in the conversation.
#[derive(Debug, Clone, PartialEq)]
pub struct ObservedToolRun {
    pub session_id: String,
    pub turn_number: u64,
    /// Position within its turn, 0-based. Used for recency, not for identity.
    pub ordinal: u32,
    pub tool_use_id: String,
    pub attempt: u32,
    pub tool_name: String,
    /// A bounded rendering of the tool input. Never the raw payload.
    pub input_summary: String,
    pub effect_class: ActionEffectClass,
    /// Deterministic: the provider marked the result an error.
    pub failed: bool,
    /// Deterministic: the run was refused before it executed.
    pub blocked: bool,
}

impl ObservedToolRun {
    /// Immutable identity of this attempt, in the shape slice 4 defines:
    /// session, provider tool-use reference, attempt ordinal. Retries get
    /// distinct ids because the attempt number is part of it.
    pub fn action_attempt_id(&self) -> String {
        format!("{}:{}:{}", self.session_id, self.tool_use_id, self.attempt)
    }
}

/// One finalized decision observed in the decision ledger.
#[derive(Debug, Clone, PartialEq)]
pub struct ObservedDecision {
    pub decision_id: String,
    pub session_id: String,
    pub turn_number: u64,
    pub selected_candidate_id: String,
    /// Coarse action kind of the selected candidate.
    ///
    /// `DecisionRecord` stores the selected candidate by id only, so the kind
    /// has to come back out of the summary it renders. See
    /// [`action_kind_from_decision_summary`].
    pub action_kind: String,
    /// The decision's user-visible summary, bounded by the caller.
    pub summary: String,
}

/// Value used when a decision summary does not carry a recoverable action kind.
pub const UNRESOLVED_ACTION_KIND: &str = "unresolved_action_kind";

/// Recover the selected action kind from a decision's user-visible summary.
///
/// `decision_codec::summary_for` renders `"{situation} -> {kind} (risk=…)"`, and
/// the kind is the only part of the selected candidate that survives into the
/// stored record. The reader lives in the same crate as the writer on purpose:
/// if that format changes, both ends are in one diff.
///
/// Returns [`UNRESOLVED_ACTION_KIND`] rather than a guess when the summary is
/// not in that shape.
pub fn action_kind_from_decision_summary(summary: &str) -> &str {
    let Some(after_arrow) = summary.split_once(" -> ").map(|(_, rest)| rest) else {
        return UNRESOLVED_ACTION_KIND;
    };
    let kind = after_arrow
        .split_once(" (")
        .map_or(after_arrow, |(kind, _)| kind)
        .trim();
    if kind.is_empty() {
        UNRESOLVED_ACTION_KIND
    } else {
        kind
    }
}

/// Everything one attribution may consider.
#[derive(Debug, Clone, PartialEq)]
pub struct AttributionInput {
    pub correction: CorrectionUnderReview,
    pub tool_runs: Vec<ObservedToolRun>,
    pub decisions: Vec<ObservedDecision>,
}

/// The correction record does not name a turn the engine can work from.
pub const UNATTRIBUTED_PROVENANCE_INCOMPLETE: &str = "unattributed.provenance_incomplete";
/// Nothing was observed in the window at all.
pub const UNATTRIBUTED_EMPTY_WINDOW: &str = "unattributed.empty_window";
/// Things were observed, but none of them belong to this correction's session
/// and lookback window.
pub const UNATTRIBUTED_NO_ELIGIBLE_CANDIDATE: &str = "unattributed.no_eligible_candidate";

impl AttributionInput {
    /// Preconditions that make an attribution impossible regardless of scoring.
    ///
    /// Returns the rationale code the assessment must carry, or `None` when the
    /// input is well-formed enough to score. Turn zero is rejected because turn
    /// numbers start at one: a zero means the caller never set one, and treating
    /// it as a real turn would make every action in the window "later than the
    /// correction" or none of them, depending on the comparison -- either way a
    /// verdict derived from a field nobody filled in.
    pub fn precondition_failure(&self) -> Option<&'static str> {
        if self.correction.correction_id.trim().is_empty()
            || self.correction.session_id.trim().is_empty()
            || self.correction.turn_number == 0
        {
            return Some(UNATTRIBUTED_PROVENANCE_INCOMPLETE);
        }
        if self.tool_runs.is_empty() && self.decisions.is_empty() {
            return Some(UNATTRIBUTED_EMPTY_WINDOW);
        }
        None
    }
}
