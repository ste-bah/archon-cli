//! Which recorded execution a session's remediation fix was replayed from,
//! so a recorded verdict follows only the fix it judged
//! (`script::resume_verdict`).
//!
//! A write's branch outcomes are saved before its call record. A session
//! killed between the two leaves a NEW answer beside the OLD record, whose
//! finish time then describes an execution that is not the one replayed.
//! So a finish time is taken only from a record no outcome of the fix's
//! label -- under any ordinal -- was written after: then every answer on
//! disk, the replayed one included, existed when that record finished, and
//! no later execution of the label left anything behind. Anything
//! unreadable proves nothing, and no verdict follows the fix.

use std::time::SystemTime;

use super::*;
use crate::v2::result_store::{ReplayedFix, WorkflowV2CallRecord};
use crate::v2::script::history_replay::call_family;
use crate::v2::script::resume_verdict::{is_remediation_fix, remediation_round_key};

/// The latest time any branch outcome filed under `call_id`'s label (the
/// fan-out's id; `call` is a branch's, which carries the contract) was
/// written, as the earlier sessions left them: read before this session
/// saves one. `None` for a call that is no remediation work, or when a
/// directory or a time cannot be read.
pub(in crate::v2::branch_cache) fn label_last_written(
    v2_store: &WorkflowV2ResultStore,
    call_id: &str,
    call: &crate::WorkflowV2HostCall,
) -> Option<SystemTime> {
    if !crate::v2::script::resume_drift::is_remediation_call(call) {
        return None;
    }
    // Directory names are the store's sanitized call ids.
    let label: String = call_family(call_id)
        .0
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect();
    let root = v2_store.root().join("branches");
    let mut latest = SystemTime::UNIX_EPOCH;
    let calls = match std::fs::read_dir(&root) {
        Ok(calls) => calls,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Some(latest),
        Err(_) => return None,
    };
    for dir in calls {
        let dir = dir.ok()?;
        let name = dir.file_name().to_string_lossy().into_owned();
        if !dir.file_type().ok()?.is_dir() || call_family(&name).0 != label {
            continue;
        }
        for entry in std::fs::read_dir(dir.path()).ok()? {
            let entry = entry.ok()?;
            if entry.file_type().ok()?.is_file() {
                latest = latest.max(entry.metadata().ok()?.modified().ok()?);
            }
        }
    }
    Some(latest)
}

/// The fix `record` answers as, replayed whole or per branch: its call id
/// and when its execution finished as the earlier sessions left it -- only
/// when nothing of its label was written after (`label_written`, read
/// before this session saved any).
fn proven(
    record: &WorkflowV2CallRecord,
    v2_store: &WorkflowV2ResultStore,
    label_written: SystemTime,
) -> Option<ReplayedFix> {
    let finished_at = v2_store.recorded_finish(record)?;
    let at = chrono::DateTime::parse_from_rfc3339(&finished_at).ok()?;
    // The label is checked against the record as saved; the pairing reads
    // the execution the record restates (Issue-111).
    let mut executed = v2_store.executed_finish(record)?;
    // A record a session re-saved (a later attempt) before records named the
    // execution they restate: its finish is the re-save's. Every answer of
    // the label, the replayed one included, was on disk by `label_written`,
    // so that bounds when the replayed execution answered.
    if record.answered_by.is_none()
        && record.attempt > 1
        && label_written > SystemTime::UNIX_EPOCH
        && label_written < SystemTime::from(at)
    {
        executed = chrono::DateTime::<chrono::Utc>::from(label_written).to_rfc3339();
    }
    (label_written <= SystemTime::from(at)).then(|| ReplayedFix {
        call_id: record.call.id.clone(),
        finished_at: executed,
    })
}

/// Whether `record` is provably the latest execution of its label: nothing
/// of the label was written after it finished.
pub(super) fn wrote_nothing_after(
    v2_store: &WorkflowV2ResultStore,
    record: &WorkflowV2CallRecord,
    label_written: Option<SystemTime>,
) -> bool {
    label_written.is_some_and(|written| proven(record, v2_store, written).is_some())
}

/// The lineage of a fix replayed whole from `record` (call-level reuse),
/// read before the host re-saves it; `None` when it cannot be proven.
pub fn replayed_fix(
    v2_store: &WorkflowV2ResultStore,
    record: &WorkflowV2CallRecord,
) -> Option<ReplayedFix> {
    let written = label_last_written(v2_store, &record.call.id, &record.call)?;
    proven(record, v2_store, written)
}

/// How one session answered a fix call's branches: the record each reused
/// branch was answered from, in order, and the drifted siblings among them
/// whose answer was re-derived exactly as already filed under the call's
/// own id (Issue-109) -- those count as the own record's.
pub(in crate::v2::branch_cache) struct Replayed<'a> {
    pub(in crate::v2::branch_cache) sources: &'a [String],
    pub(in crate::v2::branch_cache) refiled_from: &'a [String],
    pub(in crate::v2::branch_cache) none_pending: bool,
}

/// Record how this session answered a fix call's branches: replayed from
/// one record's execution when every branch was reused from it and that
/// execution is provably the one whose answers were replayed; otherwise run.
///
/// Issue-109: a fix whose branches re-derived a drifted sibling's answer
/// exactly as an earlier session filed it under this id replays THAT
/// session's record, proven like any own record: nothing of the label was
/// written after it finished. The sibling is kept as the origin its re-save
/// restates, so the next resume finds the sibling's manifest and replays the
/// fix under its own id.
pub(in crate::v2::branch_cache) fn note_fix_lineage(
    v2_store: &WorkflowV2ResultStore,
    call_id: &str,
    call: &crate::WorkflowV2HostCall,
    answered: Replayed<'_>,
    label_written: Option<SystemTime>,
) {
    if !is_remediation_fix(call) {
        return;
    }
    let Some(key) = remediation_round_key(call) else {
        return;
    };
    let sources = answered.sources;
    let single = sources
        .first()
        .filter(|first| answered.none_pending && sources.iter().all(|source| source == *first));
    let replayed = single.and_then(|source| {
        let record = v2_store.load_call_record(source).ok().flatten()?;
        proven(&record, v2_store, label_written?)
    });
    if single.is_some() && replayed.is_none() {
        eprintln!(
            "remediation replay: {} replayed an answer no recorded execution provably wrote; its verdict runs again",
            call.id
        );
    }
    let origin = answered.refiled_from.first().filter(|origin| {
        replayed.is_some()
            && single.is_some_and(|source| source == call_id)
            && answered.refiled_from.iter().all(|other| other == *origin)
    });
    v2_store.note_fix_lineage(&key, replayed);
    if let Some(origin) = origin {
        v2_store.note_refile_origin(&key, origin);
    }
}

/// The manifest of the execution `call_id`'s record restates (Issue-111): a
/// fix a session answered by refiling a drifted sibling's outcome keeps the
/// sibling's manifest under the sibling's stage, and its record names that
/// execution (`answered_by`). Only a single manifest counts: a remediation
/// write is one item.
pub(super) fn restated_manifest(
    v2_store: &WorkflowV2ResultStore,
    call_id: &str,
) -> Option<crate::write_coordinator::PatchManifest> {
    let origin = v2_store.load_call_record(call_id).ok()??.answered_by?;
    if origin.call_id == call_id {
        return None;
    }
    let dir = v2_store
        .root()
        .parent()?
        .join("write-coordination")
        .join("stages")
        .join(&origin.call_id)
        .join("manifests");
    let manifests: Vec<_> = std::fs::read_dir(dir).ok()?.flatten().collect();
    let [only] = manifests.as_slice() else {
        return None;
    };
    serde_json::from_slice(&std::fs::read(only.path()).ok()?).ok()
}

/// A replayed fix whose reused result the host then rejected (revalidation)
/// was not answered by that record: no verdict may follow it.
pub fn forget_fix_lineage(v2_store: &WorkflowV2ResultStore, call: &crate::WorkflowV2HostCall) {
    if is_remediation_fix(call)
        && let Some(key) = remediation_round_key(call)
    {
        v2_store.note_fix_lineage(&key, None);
    }
}
