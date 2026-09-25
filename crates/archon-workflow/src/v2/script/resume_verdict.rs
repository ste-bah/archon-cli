//! A replayed remediation verdict vouches for the fix it judged, and only
//! for that fix.
//!
//! A verifier's input is the findings, not the patch: a verdict recorded
//! about one fix hashes the same as a question about a different one. So a
//! verifier record -- same id, drifted or history -- may answer a verify call
//! only when THIS session's fix of the same unit and round was itself
//! replayed from the fix that verdict followed. A fix that ran again (a
//! fresh coder, or branches answered from more than one record) is judged
//! again: replaying the old verdict would count an unverified patch
//! resolved.

use super::history_replay::call_family;
use super::resume_drift::remediation_unit;
use super::{
    WorkflowV2CallRecord, WorkflowV2HostCall, WorkflowV2ResultStore, remediation_contract,
};

const FIX_STAGE: &str = "remediate";
const VERDICT_STAGE: &str = "verify";

fn stage(call: &WorkflowV2HostCall) -> Option<&str> {
    remediation_contract(call)?.get("stage")?.as_str()
}

/// The unit and round a remediation call belongs to, whatever its stage:
/// the key a fix and its verdict share.
pub fn remediation_round_key(call: &WorkflowV2HostCall) -> Option<String> {
    remediation_unit(call).map(|(unit, round)| format!("{unit}#{round}"))
}

pub fn is_remediation_fix(call: &WorkflowV2HostCall) -> bool {
    stage(call) == Some(FIX_STAGE)
}

pub fn is_remediation_verdict(call: &WorkflowV2HostCall) -> bool {
    stage(call) == Some(VERDICT_STAGE)
}

/// The fix a recorded verdict judged: the latest fix of its unit and round
/// ordered before it. Records `skip` names (this session's own) are not the
/// verdict's past.
pub fn paired_fix<'a>(
    verdict: &WorkflowV2CallRecord,
    records: &'a [WorkflowV2CallRecord],
    skip: impl Fn(&str) -> bool,
) -> Option<&'a WorkflowV2CallRecord> {
    let key = remediation_round_key(&verdict.call)?;
    let (_, Some(ordinal)) = call_family(&verdict.call.id) else {
        return None;
    };
    records
        .iter()
        .filter(|record| {
            is_remediation_fix(&record.call)
                && !skip(&record.call.id)
                && remediation_round_key(&record.call).as_deref() == Some(key.as_str())
        })
        .filter_map(|record| match call_family(&record.call.id) {
            (_, Some(fix_ordinal)) if fix_ordinal < ordinal => Some((fix_ordinal, record)),
            _ => None,
        })
        .max_by_key(|(fix_ordinal, _)| *fix_ordinal)
        .map(|(_, record)| record)
}

/// Whether `record` may answer a call in this session. Anything but a
/// remediation verdict may; a verdict only when this session's fix of its
/// unit and round was replayed from the fix it judged.
pub fn verdict_vouches_for_session_fix(
    record: &WorkflowV2CallRecord,
    records: &[WorkflowV2CallRecord],
    store: &WorkflowV2ResultStore,
) -> bool {
    if !is_remediation_verdict(&record.call) {
        return true;
    }
    let Some(key) = remediation_round_key(&record.call) else {
        return false;
    };
    let Some(source) = store.fix_replayed_from(&key) else {
        return false;
    };
    paired_fix(record, records, |id| id != source && store.in_session(id))
        .is_some_and(|fix| fix.call.id == source)
}

#[cfg(test)]
#[path = "resume_verdict_tests.rs"]
mod tests;
