use chrono::{TimeZone, Utc};

use super::*;
use crate::attribution::candidates::ATTRIBUTION_LOOKBACK_TURNS;
use crate::attribution::input::{
    ActionEffectClass, CorrectionUnderReview, ObservedDecision, ObservedToolRun,
    UNATTRIBUTED_EMPTY_WINDOW, UNATTRIBUTED_NO_ELIGIBLE_CANDIDATE,
    UNATTRIBUTED_PROVENANCE_INCOMPLETE,
};

#[path = "tests/engine.rs"]
mod engine;
#[path = "tests/event.rs"]
mod event_rows;

const SESSION: &str = "attribution-session";

fn correction(
    correction_type_code: &str,
    summary: &str,
    turn_number: u64,
) -> CorrectionUnderReview {
    CorrectionUnderReview {
        correction_id: "corr-1".into(),
        session_id: SESSION.into(),
        turn_number,
        correction_type_code: correction_type_code.into(),
        summary: summary.into(),
        recorded_at: Utc.with_ymd_and_hms(2026, 8, 10, 12, 0, 0).unwrap(),
    }
}

fn tool_run(tool_use_id: &str, tool_name: &str, turn_number: u64, ordinal: u32) -> ObservedToolRun {
    ObservedToolRun {
        session_id: SESSION.into(),
        turn_number,
        ordinal,
        tool_use_id: tool_use_id.into(),
        attempt: 1,
        tool_name: tool_name.into(),
        input_summary: String::new(),
        effect_class: ActionEffectClass::Read,
        failed: false,
        blocked: false,
    }
}

fn decision(decision_id: &str, turn_number: u64) -> ObservedDecision {
    ObservedDecision {
        decision_id: decision_id.into(),
        session_id: SESSION.into(),
        turn_number,
        selected_candidate_id: "cand-1".into(),
        action_kind: "InspectFiles".into(),
        summary: "look at the failing module".into(),
    }
}

fn input(
    correction: CorrectionUnderReview,
    tool_runs: Vec<ObservedToolRun>,
    decisions: Vec<ObservedDecision>,
) -> AttributionInput {
    AttributionInput {
        correction,
        tool_runs,
        decisions,
    }
}

fn attribute(input: &AttributionInput) -> AttributionAssessment {
    AttributionEngine.attribute(input)
}
