//! Issue-122: where a host-planned unit's calls sat in the prelude's call
//! ordinal, so a resumed session files them where they were filed.
//!
//! The prelude names every `agent()` call `<label>-<ordinal>`, one global
//! ordinal per session. A host-planned unit -- a residual round, a contest
//! remediation -- that a resumed session SKIPS (recorded done) makes no call,
//! so the ordinal never advances past it: the next unit's calls get lower
//! numbers than the ones the host recorded, miss the store, and re-dispatch
//! a write that already landed (on a live run: a second-pass round
//! recorded as `…-81` was re-dispatched as `…-79`).
//!
//! The unit's own records answer where it sat. Its calls carry its key in
//! their remediation contract (`contest`, the prelude's unit key); each
//! attempt of the unit begins with its round-1 fix. For the unit's latest
//! attempt (its latest-started round-1 fix and every record of the unit
//! started from then on):
//!
//! - `fix_ordinal`: that fix's ordinal -- the prelude files an unfinished
//!   unit's first fix under it, so it replays;
//! - `resume_ordinal`: the highest ordinal the attempt used, counting the
//!   `log()` the prelude makes right after a fix that landed nothing -- the
//!   prelude continues from it past a unit it skips.
//!
//! Ids the old preludes recorded are exactly these numbers, so aligning to
//! them reproduces the recording session's ids; a unit with no record
//! changes nothing. The ordinal is never moved below where the prelude's
//! unit loop began, so no later call can take an earlier call's id.

use serde_json::Value;

use super::{WorkflowV2CallRecord, WorkflowV2HostMethod, remediation_contract};

/// Where one unit's latest attempt sat in the prelude's ordinal.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UnitOrdinals {
    pub fix_ordinal: Option<u64>,
    pub resume_ordinal: Option<u64>,
}

/// The ordinal a prelude call id ends in (`<label>-<n>`), if any.
pub fn ordinal_of(id: &str) -> Option<u64> {
    let (_, tail) = id.rsplit_once('-')?;
    (!tail.is_empty() && tail.bytes().all(|b| b.is_ascii_digit()))
        .then(|| tail.parse().ok())
        .flatten()
}

fn unit_of(record: &WorkflowV2CallRecord) -> Option<&str> {
    remediation_contract(&record.call)
        .and_then(|contract| contract.get("contest"))
        .and_then(Value::as_str)
}

fn stage_of(record: &WorkflowV2CallRecord) -> Option<&str> {
    remediation_contract(&record.call)
        .and_then(|contract| contract.get("stage"))
        .and_then(Value::as_str)
}

fn started(record: &WorkflowV2CallRecord) -> i64 {
    chrono::DateTime::parse_from_rfc3339(&record.started_at)
        .ok()
        .and_then(|at| at.timestamp_nanos_opt())
        .unwrap_or(i64::MIN)
}

fn round_of(record: &WorkflowV2CallRecord) -> Option<u64> {
    remediation_contract(&record.call)
        .and_then(|contract| contract.get("round"))
        .and_then(Value::as_u64)
}

/// The ordinals of the unit keyed `unit` (its contract's `contest`) in
/// `records`.
pub fn unit_ordinals(records: &[WorkflowV2CallRecord], unit: &str) -> UnitOrdinals {
    let own: Vec<&WorkflowV2CallRecord> = records
        .iter()
        .filter(|record| unit_of(record) == Some(unit))
        .collect();
    let is_fix = |record: &WorkflowV2CallRecord| {
        stage_of(record) == Some("remediate")
            && record.call.method != WorkflowV2HostMethod::Checkpoint
    };
    let Some(first) = own
        .iter()
        .copied()
        .filter(|record| is_fix(record) && round_of(record) == Some(1))
        .filter(|record| ordinal_of(&record.call.id).is_some())
        .max_by_key(|record| started(record))
    else {
        return UnitOrdinals::default();
    };
    let attempt: Vec<&WorkflowV2CallRecord> = own
        .iter()
        .copied()
        .filter(|record| started(record) >= started(first))
        .collect();
    let mut last = ordinal_of(&first.call.id);
    for record in &attempt {
        if let Some(n) = ordinal_of(&record.call.id) {
            last = last.max(Some(n));
        }
        // A fix that landed nothing is followed by one `log()`, which takes
        // the next ordinal; its round's no-patch checkpoint records that.
        if record.call.method == WorkflowV2HostMethod::Checkpoint
            && stage_of(record) == Some("verify")
            && let Some(round) = round_of(record)
            && let Some(fix) = attempt
                .iter()
                .filter(|fix| is_fix(fix) && round_of(fix) == Some(round))
                .filter(|fix| started(fix) <= started(record))
                .max_by_key(|fix| started(fix))
            && let Some(n) = ordinal_of(&fix.call.id)
        {
            last = last.max(Some(n + 1));
        }
    }
    UnitOrdinals {
        fix_ordinal: ordinal_of(&first.call.id),
        resume_ordinal: last,
    }
}

#[cfg(test)]
#[path = "resume_ordinals_tests.rs"]
mod tests;
