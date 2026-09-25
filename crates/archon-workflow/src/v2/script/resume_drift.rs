//! Resume reuse for review remediation: ordinal drift and superseded rounds.
//!
//! The prelude names every call `slug(label)-<global ordinal>`, and the
//! ordinal counts every `agent()`/`log()` the script has made. When one
//! task's remediation takes a different number of calls on a resume -- a
//! round that landed no patch now lands one and is verified in round 1 --
//! every later call arrives under a shifted id, and a store keyed on the id
//! finds nothing for work it already holds. Two rules recover it, both keyed
//! on content, never on the id:
//!
//! * **Drift.** A remediation call (it carries a `remediationContract`) may
//!   replay a record of the same label under another ordinal when its input,
//!   rewritten to that ordinal, hashes to exactly the recorded input hash.
//!   The input carries the prompt (the verbatim findings) and the contract
//!   (task, round), so a different round, task or finding set never matches
//!   -- which matters, because `slug()` cuts a label at 40 characters and a
//!   long unit key (every cross-task unit) loses its round from the label.
//! * **History.** A remediation record that a later round of the same unit
//!   has superseded is the answer the script already acted on. Replaying it
//!   verbatim -- whatever its status -- walks the script down the path it
//!   recorded; asking again gets a different answer from a non-deterministic
//!   agent and re-runs every round after it. A last round is never history,
//!   nor is a later round recorded before the review it remediates last ran,
//!   nor a record the host got no answer for (transport, interruption).
//!
//! Both answer only from EARLIER sessions, and each record at most once: a
//! record this session wrote or already replayed is never a candidate
//! (`WorkflowV2ResultStore::in_session`). The same label with the same input
//! inside one run is deliberate -- acceptance re-asks a fix whose check
//! still fails, a transport retry re-asks a dead call -- and must run.

use serde_json::Value;

use super::history_replay::call_family;
use super::{
    WorkflowV2CallExecution, WorkflowV2CallRecord, WorkflowV2HostCall, WorkflowV2Status,
    is_reusable_status, is_transport_failure_text, remediation_contract,
    reusable_record_has_required_completion_evidence,
};

/// The host's per-task verifier wave prefix: the prelude files a verifier as
/// `verification-wave-{id}` and its item as `{id}-check`, so the ordinal
/// token the ids share is the part after this prefix.
const VERIFICATION_WAVE_PREFIX: &str = "verification-wave-";

/// Whether a call is remediation work a review (or acceptance) asked for.
pub fn is_remediation_call(call: &WorkflowV2HostCall) -> bool {
    remediation_contract(call).is_some()
}

/// Whether two calls carry the same remediation contract -- stage, round,
/// task(s), budget and source reviews. Two rounds of one unit can share a
/// label the prelude's `slug()` cut short; the contract never does.
pub fn same_remediation_contract(left: &WorkflowV2HostCall, right: &WorkflowV2HostCall) -> bool {
    matches!(
        (remediation_contract(left), remediation_contract(right)),
        (Some(left), Some(right)) if left == right
    )
}

/// The unit a remediation call belongs to and its round. Two calls are rounds
/// of the same unit only when the whole contract other than stage and round
/// agrees.
pub(super) fn remediation_unit(call: &WorkflowV2HostCall) -> Option<(String, u64)> {
    let contract = remediation_contract(call)?;
    let round = contract.get("round").and_then(Value::as_u64)?;
    let key = serde_json::json!({
        "version": contract.get("version"),
        "taskId": contract.get("taskId"),
        "taskIds": contract.get("taskIds"),
        "maxRounds": contract.get("maxRounds"),
        "sourceReduceCallIds": contract.get("sourceReduceCallIds"),
    });
    Some((key.to_string(), round))
}

/// When the reviews a remediation contract acts on were last recorded: a
/// later round from before then answered a different finding set.
fn sources_recorded_at<'a>(
    call: &WorkflowV2HostCall,
    records: &'a [WorkflowV2CallRecord],
) -> &'a str {
    let sources = remediation_contract(call)
        .and_then(|contract| contract.get("sourceReduceCallIds"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    records
        .iter()
        .filter(|record| {
            sources
                .iter()
                .any(|id| id.as_str() == Some(record.call.id.as_str()))
        })
        .map(|record| record.started_at.as_str())
        .max()
        .unwrap_or("")
}

/// A remediation record a later round of its unit has superseded, the later
/// round recorded after the reviews it remediates.
pub fn superseded_remediation_record(
    record: &WorkflowV2CallRecord,
    records: &[WorkflowV2CallRecord],
) -> bool {
    let Some((key, round)) = remediation_unit(&record.call) else {
        return false;
    };
    let sources_at = sources_recorded_at(&record.call, records);
    record.invalidated_by.is_none()
        && records.iter().any(|other| {
            other.call.id != record.call.id
                && other.invalidated_by.is_none()
                && other.started_at.as_str() > sources_at
                && remediation_unit(&other.call)
                    .is_some_and(|(other_key, other_round)| other_key == key && other_round > round)
        })
}

/// A call record that carries an agent's answer: not cancelled, not a
/// transport failure, and every branch it fanned out to answered.
fn answered_record(record: &WorkflowV2CallRecord) -> bool {
    let branches = record
        .result
        .data
        .get("outcomes")
        .and_then(Value::as_array)
        .filter(|views| !views.is_empty());
    record.status != WorkflowV2Status::Cancelled
        && record.result.validate().is_ok()
        && !is_transport_failure_text(&record.result.summary)
        && branches.is_some_and(|views| {
            views.iter().all(|view| {
                view.get("result").is_some_and(Value::is_object)
                    && view.get("error").is_none_or(Value::is_null)
                    && !matches!(
                        view.get("failure_kind").and_then(Value::as_str),
                        Some("execution" | "safety")
                    )
            })
        })
}

/// `(token, ordinal)` for a prelude-minted id: `("review-remediate-t-1",
/// "31")` for `review-remediate-t-1-31`, and the part after the verifier
/// wave prefix for `verification-wave-review-verify-t-1-32`.
pub fn ordinal_token(call_id: &str) -> Option<(&str, &str)> {
    let (label, Some(_)) = call_family(call_id) else {
        return None;
    };
    let ordinal = &call_id[label.len() + 1..];
    let token = label
        .strip_prefix(VERIFICATION_WAVE_PREFIX)
        .unwrap_or(label);
    (!token.is_empty()).then_some((token, ordinal))
}

/// `text` with every whole `{token}-{from}` rewritten to `{token}-{to}`. A
/// match followed by another digit is a different ordinal and is kept.
pub fn rebase_text(text: &str, token: &str, from: &str, to: &str) -> String {
    let needle = format!("{token}-{from}");
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(position) = rest.find(&needle) {
        let end = position + needle.len();
        let whole = rest[end..]
            .chars()
            .next()
            .is_none_or(|next| !next.is_ascii_digit());
        out.push_str(&rest[..position]);
        if whole {
            out.push_str(token);
            out.push('-');
            out.push_str(to);
        } else {
            out.push_str(&needle);
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

/// Every string in `value` rebased with [`rebase_text`]; keys are kept.
pub fn rebase_value(value: &Value, token: &str, from: &str, to: &str) -> Value {
    match value {
        Value::String(text) => Value::String(rebase_text(text, token, from, to)),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| rebase_value(item, token, from, to))
                .collect(),
        ),
        Value::Object(object) => Value::Object(
            object
                .iter()
                .map(|(key, item)| (key.clone(), rebase_value(item, token, from, to)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// The execution as the script would have issued it under `candidate_id`'s
/// ordinal, when both ids are the same label.
pub fn rebase_execution(
    execution: &WorkflowV2CallExecution,
    candidate_id: &str,
) -> Option<WorkflowV2CallExecution> {
    let (token, from) = ordinal_token(&execution.call.id)?;
    let (candidate_token, to) = ordinal_token(candidate_id)?;
    if token != candidate_token
        || from == to
        || call_family(&execution.call.id).0 != call_family(candidate_id).0
        || rebase_text(&execution.call.id, token, from, to) != candidate_id
    {
        return None;
    }
    let mut rebased = execution.clone();
    rebased.call.id = candidate_id.to_string();
    rebased.input = rebase_value(&execution.input, token, from, to);
    Some(rebased)
}

/// Same-label remediation records under another ordinal from earlier
/// sessions (`in_session` names what this one wrote or replayed), lowest
/// ordinal first so the n-th call of a label meets the n-th record of it.
pub fn drift_candidates<'a>(
    call_id: &str,
    records: &'a [WorkflowV2CallRecord],
    in_session: impl Fn(&str) -> bool,
) -> Vec<&'a WorkflowV2CallRecord> {
    let (label, Some(_)) = call_family(call_id) else {
        return Vec::new();
    };
    let mut candidates: Vec<(u64, &WorkflowV2CallRecord)> = records
        .iter()
        .filter(|record| {
            record.call.id != call_id
                && record.invalidated_by.is_none()
                && is_remediation_call(&record.call)
                && !in_session(&record.call.id)
        })
        .filter_map(|record| match call_family(&record.call.id) {
            (other, Some(ordinal)) if other == label => Some((ordinal, record)),
            _ => None,
        })
        .collect();
    candidates.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then_with(|| left.1.call.id.cmp(&right.1.call.id))
    });
    candidates.into_iter().map(|(_, record)| record).collect()
}

/// Whether the operator restarted this very call: its own record is
/// invalidated. A restart asks for the work again, so neither a sibling nor
/// history may answer for it.
pub fn restarted(call_id: &str, records: &[WorkflowV2CallRecord]) -> bool {
    records
        .iter()
        .any(|record| record.call.id == call_id && record.invalidated_by.is_some())
}

/// A record the host may replay as the answer to a remediation call it holds
/// no reusable record for. `in_session(id)` names records this session wrote
/// or replayed; `matches(execution, record)` is the host's strict content
/// check (input hash, source fingerprint, scaffold) of `execution` against
/// `record`. Reusable records win over history.
pub fn remediation_replay_record<'a>(
    execution: &WorkflowV2CallExecution,
    records: &'a [WorkflowV2CallRecord],
    in_session: impl Fn(&str) -> bool,
    matches: impl Fn(&WorkflowV2CallExecution, &WorkflowV2CallRecord) -> bool,
) -> Option<&'a WorkflowV2CallRecord> {
    if !is_remediation_call(&execution.call) || restarted(&execution.call.id, records) {
        return None;
    }
    let reusable = |record: &WorkflowV2CallRecord| {
        is_reusable_status(record.status)
            && reusable_record_has_required_completion_evidence(record)
    };
    let history = |record: &WorkflowV2CallRecord| {
        answered_record(record) && superseded_remediation_record(record, records)
    };
    let candidates = drift_candidates(&execution.call.id, records, &in_session);
    let drifted = |wanted: &dyn Fn(&WorkflowV2CallRecord) -> bool| {
        candidates.iter().copied().find(|record| {
            wanted(record)
                && rebase_execution(execution, &record.call.id)
                    .is_some_and(|rebased| matches(&rebased, record))
        })
    };
    if let Some(record) = drifted(&reusable) {
        return Some(record);
    }
    let own = records
        .iter()
        .find(|record| record.call.id == execution.call.id)
        .filter(|record| {
            !is_reusable_status(record.status) && history(record) && matches(execution, record)
        });
    own.or_else(|| drifted(&history))
}

#[cfg(test)]
#[path = "resume_drift_tests.rs"]
mod tests;
