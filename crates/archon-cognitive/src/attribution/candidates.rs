//! Candidate generation: which prior actions and decisions could have caused
//! this correction at all.
//!
//! Generation is deliberately a filter, not a search. Everything eligible
//! becomes a candidate and scoring decides between them, so the population the
//! confidence is computed over is the population that was actually considered.
//! A generator that pre-selected "the likely ones" would make every confidence
//! conditional on a step nothing records.

use crate::attribution::CauseActionClass;
use crate::attribution::input::{ActionEffectClass, AttributionInput};

/// How many turns back an action may be and still be a candidate.
///
/// A user correcting an action usually does it on the very next turn, which is
/// distance 1. Two allows for the case where the agent answered once in between
/// without acting. Beyond that the link stops being a link: an action six turns
/// back competes with everything since, and "it was in the window" is not
/// evidence.
pub const ATTRIBUTION_LOOKBACK_TURNS: u64 = 2;

/// Upper bound on the candidate population.
///
/// Scoring is linear, so this is not about cost -- it bounds how much a single
/// tool-heavy turn can dominate a measurement row's identity fields. When the
/// window is bigger than this, the MOST RECENT actions are kept, because those
/// are the ones a correction is about.
pub const MAX_CANDIDATES: usize = 12;

/// One thing that could have caused the correction.
#[derive(Debug, Clone, PartialEq)]
pub struct CausalCandidate {
    /// Deterministic identity, so a retried attribution proposes the same
    /// candidate id and the metric store recognises the replay.
    pub candidate_id: String,
    pub cause_action_class: CauseActionClass,
    /// Human-readable name of the action: the tool name, or the decision's
    /// candidate action kind.
    pub label: String,
    pub session_id: String,
    pub turn_number: u64,
    pub ordinal: u32,
    pub tool_use_id: Option<String>,
    pub action_attempt_id: Option<String>,
    pub decision_id: Option<String>,
    pub effect_class: ActionEffectClass,
    pub failed: bool,
    pub blocked: bool,
    /// Text the lexical evidence is computed over.
    pub text: String,
}

impl CausalCandidate {
    /// How many turns before the correction this candidate sits.
    pub fn turn_distance(&self, correction_turn: u64) -> u64 {
        correction_turn.saturating_sub(self.turn_number)
    }

    /// Typed join keys for this candidate, recorded as event evidence.
    pub fn evidence_refs(&self) -> Vec<String> {
        let mut refs = vec![format!("turn:{}", self.turn_number)];
        if let Some(tool_use_id) = &self.tool_use_id {
            refs.push(format!("tool_use:{tool_use_id}"));
        }
        if let Some(decision_id) = &self.decision_id {
            refs.push(format!("decision:{decision_id}"));
        }
        if self.failed {
            refs.push("tool_result:is_error".to_string());
        }
        if self.blocked {
            refs.push("tool_run:blocked".to_string());
        }
        refs
    }
}

/// Whether an observation belongs to this correction's session and window.
///
/// Session equality is the wrong-session guard the R2 rollback trigger names
/// ("any causal lesson linked to wrong session/action"). It is checked here
/// rather than trusted from the caller because the caller reads two different
/// stores, and a session filter that lives in the query is a filter that is
/// silently absent the day someone adds a third reader.
fn eligible(
    correction_session: &str,
    correction_turn: u64,
    session_id: &str,
    turn_number: u64,
) -> bool {
    if session_id != correction_session {
        return false;
    }
    // Strictly prior: the correction arrived at the start of its own turn, so
    // anything recorded against that turn or later happened after the user had
    // already said what was wrong and cannot be what they were correcting.
    if turn_number >= correction_turn {
        return false;
    }
    correction_turn - turn_number <= ATTRIBUTION_LOOKBACK_TURNS
}

fn candidate_id(correction_id: &str, class: CauseActionClass, key: &str) -> String {
    format!("cc:{correction_id}:{}:{key}", class.as_code())
}

/// Generate every eligible candidate for this correction.
pub fn generate(input: &AttributionInput) -> Vec<CausalCandidate> {
    let correction_session = input.correction.session_id.as_str();
    let correction_turn = input.correction.turn_number;
    let correction_id = input.correction.correction_id.as_str();

    let mut candidates: Vec<CausalCandidate> = Vec::new();

    for run in &input.tool_runs {
        if !eligible(
            correction_session,
            correction_turn,
            &run.session_id,
            run.turn_number,
        ) {
            continue;
        }
        candidates.push(CausalCandidate {
            candidate_id: candidate_id(correction_id, CauseActionClass::ToolRun, &run.tool_use_id),
            cause_action_class: CauseActionClass::ToolRun,
            label: run.tool_name.clone(),
            session_id: run.session_id.clone(),
            turn_number: run.turn_number,
            ordinal: run.ordinal,
            tool_use_id: Some(run.tool_use_id.clone()),
            action_attempt_id: Some(run.action_attempt_id()),
            decision_id: None,
            effect_class: run.effect_class,
            failed: run.failed,
            blocked: run.blocked,
            text: format!("{} {}", run.tool_name, run.input_summary),
        });
    }

    for decision in &input.decisions {
        if !eligible(
            correction_session,
            correction_turn,
            &decision.session_id,
            decision.turn_number,
        ) {
            continue;
        }
        candidates.push(CausalCandidate {
            candidate_id: candidate_id(
                correction_id,
                CauseActionClass::Decision,
                &decision.decision_id,
            ),
            cause_action_class: CauseActionClass::Decision,
            label: decision.action_kind.clone(),
            session_id: decision.session_id.clone(),
            turn_number: decision.turn_number,
            // A decision precedes every action in its turn.
            ordinal: 0,
            tool_use_id: None,
            action_attempt_id: None,
            decision_id: Some(decision.decision_id.clone()),
            effect_class: ActionEffectClass::Unknown,
            failed: false,
            blocked: false,
            text: format!("{} {}", decision.action_kind, decision.summary),
        });
    }

    // Newest first, then by ordinal within a turn, then by id so the order is
    // total and a replay produces the same ranks.
    candidates.sort_by(|left, right| {
        right
            .turn_number
            .cmp(&left.turn_number)
            .then_with(|| right.ordinal.cmp(&left.ordinal))
            .then_with(|| left.candidate_id.cmp(&right.candidate_id))
    });
    candidates.truncate(MAX_CANDIDATES);
    candidates
}

/// Number of eligible tool runs in the generated population.
///
/// A window containing exactly one action is qualitatively different from one
/// containing five: there is nothing to confuse it with. Scoring uses this, so
/// it is computed once here over the same population the ranks come from.
pub fn sole_tool_run(candidates: &[CausalCandidate]) -> Option<&str> {
    let mut tool_runs = candidates
        .iter()
        .filter(|candidate| candidate.cause_action_class == CauseActionClass::ToolRun);
    let first = tool_runs.next()?;
    if tool_runs.next().is_some() {
        return None;
    }
    Some(first.candidate_id.as_str())
}
