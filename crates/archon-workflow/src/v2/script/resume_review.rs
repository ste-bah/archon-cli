//! A finished read-only review is a result, whatever verdict it carries.
//!
//! A review map branch that read its task and returned findings reports
//! `needs_review` — that IS the review's answer, not a failure of the review.
//! Reuse used to accept only `accepted`/`noop`, so every branch that found
//! something was dispatched again on every resume. A reviewer asked the same
//! question twice answers differently, so the map's finding set moved, the
//! reducers' input hashes moved with it, and every remediation downstream of
//! them re-ran: a mid-run deploy cost the whole review phase again.
//!
//! What stays out: a branch the host could not get an answer from (execution,
//! contract, safety failures carry their own `failure_kind`, or no result at
//! all), and anything that is not a read-only review map. The call is
//! recognised by its `reviewContract` stage, never by its id.
//!
//! Replayed and fresh hand the script the same findings. The host attaches
//! and normalises a review call's findings once, before the record is
//! persisted, and the fresh path returns the view of that persisted record; a
//! replay returns the same record's view and never re-normalises it. A record
//! an older host attached therefore replays in the shape every downstream
//! record (the reduce, each remediation prompt) was keyed on -- re-normalising
//! it would move all of their input hashes on the first resume after a
//! deploy. Only a review call that genuinely re-executes produces the new
//! shape, and then everything downstream of it is new work anyway.

use serde_json::Value;

use super::{
    REVIEW_MAP_STAGE, WorkflowV2CallRecord, WorkflowV2HostCall, WorkflowV2Result, WorkflowV2Status,
    review_contract, review_contract_stage,
};
use crate::v2::review_findings::HOST_REVIEW_FINDINGS_KEY;
use crate::v2::scheduler::{BranchFailureKind, WorkflowV2BranchOutcome};

/// Where a review contract that names no `findingsPath` keeps its findings.
const DEFAULT_FINDINGS_PATH: &str = "data.findings";

/// A read-only fan-out whose review contract names the `map` stage.
pub fn is_review_map_call(call: &WorkflowV2HostCall) -> bool {
    call.write_mode.is_none()
        && review_contract_stage(call).map(str::trim) == Some(REVIEW_MAP_STAGE)
}

fn findings_path(call: &WorkflowV2HostCall) -> String {
    review_contract(call)
        .and_then(|contract| contract.get("findingsPath"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .unwrap_or(DEFAULT_FINDINGS_PATH)
        .to_string()
}

/// The value at a dotted path inside a serialized result.
fn value_at<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.')
        .filter(|segment| !segment.is_empty())
        .try_fold(value, |current, segment| current.get(segment))
}

fn result_carries_findings(result: &WorkflowV2Result, path: &str) -> bool {
    let found = match path.strip_prefix("data.") {
        Some(rest) => value_at(&result.data, rest).map(Value::is_array),
        None => serde_json::to_value(result)
            .ok()
            .and_then(|value| value_at(&value, path).map(Value::is_array)),
    };
    found.unwrap_or(false)
}

/// A review map branch that completed its review and returned findings: the
/// reviewer's own `needs_review` verdict (`failure_kind` semantic or none),
/// a result that agrees with it and validates, the findings array the
/// contract names, and the hash of the input it reviewed.
pub fn completed_review_branch(
    call: &WorkflowV2HostCall,
    outcome: &WorkflowV2BranchOutcome,
) -> bool {
    is_review_map_call(call)
        && outcome.status == WorkflowV2Status::NeedsReview
        && matches!(
            outcome.failure_kind,
            None | Some(BranchFailureKind::Semantic)
        )
        && outcome.error.is_none()
        && outcome.item_input_hash.is_some()
        && outcome.result.as_ref().is_some_and(|result| {
            result.status == outcome.status
                && result.validate().is_ok()
                && result_carries_findings(result, &findings_path(call))
        })
}

/// One branch view on a map call's recorded result, judged by the same rule
/// as a stored branch outcome. The view is the serialized outcome, so its
/// `failure_kind` is the enum's snake_case name.
fn completed_branch_view(view: &Value, path: &str) -> bool {
    let status = view
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let failure_kind = view.get("failure_kind").and_then(Value::as_str);
    let error_free = view.get("error").is_none_or(Value::is_null);
    let Some(result) = view.get("result").filter(|result| result.is_object()) else {
        return false;
    };
    let valid = serde_json::from_value::<WorkflowV2Result>(result.clone())
        .is_ok_and(|parsed| parsed.validate().is_ok());
    if !error_free || !valid || result.get("status").and_then(Value::as_str) != Some(status) {
        return false;
    }
    match status {
        "accepted" | "noop" => failure_kind.is_none(),
        "needs_review" => {
            matches!(failure_kind, None | Some("semantic"))
                && value_at(result, path).is_some_and(Value::is_array)
        }
        _ => false,
    }
}

/// A review map call whose every branch completed its review, recorded with
/// the host's finding attachment. Its aggregate `needs_review` only says some
/// reviewer found something; the record is as final as an accepted one.
pub fn completed_review_map_record(record: &WorkflowV2CallRecord) -> bool {
    if !is_review_map_call(&record.call)
        || record.status != WorkflowV2Status::NeedsReview
        || record.result.status != WorkflowV2Status::NeedsReview
    {
        return false;
    }
    let attached = record
        .result
        .data
        .get(HOST_REVIEW_FINDINGS_KEY)
        .and_then(|value| value.get("findings"))
        .is_some_and(Value::is_array);
    let path = findings_path(&record.call);
    let Some(outcomes) = record.result.data.get("outcomes").and_then(Value::as_array) else {
        return false;
    };
    attached
        && !outcomes.is_empty()
        && outcomes
            .iter()
            .all(|view| completed_branch_view(view, &path))
}

#[cfg(test)]
#[path = "resume_review_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "resume_replay_tests.rs"]
mod replay_tests;
