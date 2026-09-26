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

/// Whether `record` may answer a call in this session. Anything but a
/// remediation verdict may. A verdict only when this session's fix of its
/// unit and round was replayed from the fix it judged: the replayed fix
/// finished before the verdict, and no other recorded fix of that unit and
/// round finished after the replayed one (a later one -- before the verdict
/// or after it -- is what the verdict judged, or makes it stale). Pairing is
/// by finish time, as the earlier sessions recorded it; ordinals move both
/// ways across sessions. This session's own new records are not the
/// verdict's past.
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
    // When the replayed fix finished is the finish of the execution whose
    // answer was replayed, proven when the lineage was noted -- never the
    // finish of whatever record now sits under the fix's id.
    let Some(replayed) = store.fix_replayed(&key) else {
        return false;
    };
    let (source, source_at) = (replayed.call_id, replayed.finished_at);
    let Some(verdict_at) = store.recorded_finish(record) else {
        return false;
    };
    source_at < verdict_at
        && records
            .iter()
            .filter(|fix| {
                fix.call.id != source
                    && is_remediation_fix(&fix.call)
                    && remediation_round_key(&fix.call).as_deref() == Some(key.as_str())
            })
            .filter_map(|fix| store.recorded_finish(fix))
            .all(|finished| finished <= source_at)
}

#[cfg(test)]
#[path = "resume_verdict_tests.rs"]
mod tests;
