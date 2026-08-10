use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{CandidateActionKind, SituationKind};

/// Share of surprise attributed to the shadow loop picking a different action
/// class than the live agent took. The largest single term: it is the only one
/// that measures the planner itself rather than how the turn went.
pub const ACTION_MISMATCH_WEIGHT: f32 = 0.5;

/// Share attributed to the turn not completing. The shadow loop always plans
/// for a completed turn, so a turn that did not complete is a mismatch with
/// what it assumed even when it picked the same action.
pub const OUTCOME_MISMATCH_WEIGHT: f32 = 0.3;

/// Share attributed to failing tool calls, scaled by
/// [`MAX_COUNTED_TOOL_FAILURES`].
pub const TOOL_FAILURE_WEIGHT: f32 = 0.2;

/// Failure count at which the tool-failure term saturates.
///
/// Bounded so one pathological turn with fifty failures cannot dominate the
/// surprise distribution the reflection trigger reads.
pub const MAX_COUNTED_TOOL_FAILURES: u32 = 3;

/// A shadow plan recorded before the live turn ran.
///
/// Nothing here was executed. `selected_action` is what the executive loop
/// *would* have chosen; the live agent retained execution authority throughout.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShadowObservation {
    pub shadow_decision_id: String,
    pub session_id: String,
    pub turn_number: u64,
    pub decision_id: String,
    pub situation_id: String,
    pub situation_kind: SituationKind,
    pub selected_action: Option<CandidateActionKind>,
    pub candidate_id: String,
    /// 1-based rank of the selected candidate among the scored candidates.
    pub candidate_rank: u64,
    pub degraded: Vec<String>,
    pub created_at: DateTime<Utc>,
}

/// What the live turn actually did, supplied by the caller after finalisation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LiveTurnOutcome {
    /// Identity of the live attempt this observation is joined to.
    pub live_action_id: String,
    /// `None` when the caller could not classify what the turn did. Absence is
    /// recorded as absence, never collapsed onto "the shadow was wrong".
    pub observed_action: Option<CandidateActionKind>,
    pub completed: bool,
    pub tool_failures: u32,
    pub user_corrected: bool,
}

impl LiveTurnOutcome {
    pub fn outcome_status(&self) -> &'static str {
        if self.completed {
            "completed"
        } else {
            "not_completed"
        }
    }
}

/// Result of joining a shadow plan to the live turn that superseded it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShadowComparison {
    pub shadow_decision_id: String,
    pub decision_id: String,
    pub situation_kind: SituationKind,
    pub shadow_action: Option<CandidateActionKind>,
    pub live_action: Option<CandidateActionKind>,
    /// `None` when the live action class was not observable, which is not the
    /// same claim as "the shadow disagreed".
    pub agreed: Option<bool>,
    /// `None` for the same reason: with no comparison there is no surprise.
    pub surprise: Option<f32>,
    pub metric_recorded: bool,
}

/// Surprise of a live turn relative to the shadow plan, in `[0, 1]`.
///
/// Deliberately measured against the live turn rather than a world model, so it
/// exists before any model is validated and needs no prediction backend.
pub fn surprise_of(shadow: Option<CandidateActionKind>, live: &LiveTurnOutcome) -> Option<f32> {
    let observed = live.observed_action?;
    let mut surprise = 0.0_f32;
    if shadow != Some(observed) {
        surprise += ACTION_MISMATCH_WEIGHT;
    }
    if !live.completed {
        surprise += OUTCOME_MISMATCH_WEIGHT;
    }
    surprise += TOOL_FAILURE_WEIGHT
        * (live.tool_failures.min(MAX_COUNTED_TOOL_FAILURES) as f32
            / MAX_COUNTED_TOOL_FAILURES as f32);
    Some(surprise.clamp(0.0, 1.0))
}

/// Classify what a turn did from the tools it called.
///
/// One place, in this crate, so the shadow plan and the live observation are
/// expressed in the same vocabulary; a caller mapping tool names ad hoc would
/// make the agreement rate measure the mapping instead of the planner.
///
/// Ordering is by cost: a turn that ran tests is a test turn even if it also
/// read files, because the test run is the action the plan would have had to
/// authorise.
pub fn observed_action_from_tools(tool_names: &[String]) -> Option<CandidateActionKind> {
    if tool_names.is_empty() {
        return Some(CandidateActionKind::AnswerDirectly);
    }
    let mut observed = None;
    for name in tool_names {
        let kind = action_for_tool(name)?;
        observed = Some(match observed {
            None => kind,
            Some(previous) => dominant(previous, kind),
        });
    }
    observed
}

/// `None` for a tool this crate has no mapping for: an unmapped tool must not
/// be silently folded into `AnswerDirectly`, because that would report agreement
/// the observation does not support.
fn action_for_tool(name: &str) -> Option<CandidateActionKind> {
    Some(match name {
        "Bash" | "PowerShell" | "BashOutput" | "KillShell" => {
            CandidateActionKind::RunSafeShellProbe
        }
        "Read" | "Glob" | "Grep" | "Edit" | "Write" | "NotebookEdit" => {
            CandidateActionKind::InspectFiles
        }
        "WebFetch" | "WebSearch" | "Skill" | "ToolSearch" => CandidateActionKind::SearchDocs,
        "Task" | "Agent" | "SendMessage" => CandidateActionKind::RecallMemory,
        "ExitPlanMode" | "TodoWrite" => CandidateActionKind::AnswerDirectly,
        "AskUserQuestion" => CandidateActionKind::AskClarification,
        _ => return None,
    })
}

/// Rank used when two tools in one turn map to different classes.
fn dominant(left: CandidateActionKind, right: CandidateActionKind) -> CandidateActionKind {
    if cost_rank(right) > cost_rank(left) {
        right
    } else {
        left
    }
}

fn cost_rank(kind: CandidateActionKind) -> u8 {
    match kind {
        CandidateActionKind::AnswerDirectly => 0,
        CandidateActionKind::AskClarification => 1,
        CandidateActionKind::RecallMemory => 2,
        CandidateActionKind::SearchDocs => 3,
        CandidateActionKind::InspectFiles => 4,
        CandidateActionKind::RunSafeShellProbe => 5,
        CandidateActionKind::RunTests => 6,
        CandidateActionKind::RunLearningTick => 7,
        CandidateActionKind::CreateGovernedProposal => 8,
        CandidateActionKind::DeferOrDecline => 9,
    }
}

pub(crate) fn now() -> DateTime<Utc> {
    Utc::now()
}
