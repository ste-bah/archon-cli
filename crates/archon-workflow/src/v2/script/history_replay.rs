//! Which recorded calls are history on a resume.
//!
//! A resumed run replays its script from the top. A record that a later
//! record of the same subject has already superseded is history: re-executing
//! it asks a non-deterministic model or judge the same question again and
//! gets a different answer, spends the phase's attempt budget a second time,
//! and can lose an outcome the run had already accepted. The last record of
//! a subject is not history -- it may have been in flight when the run
//! stopped, and it is what the artifact on disk must match -- so it keeps
//! every live check. Nothing here knows any phase, PRD or provider by name:
//! an agent subject is a call-id family, a host-command subject is the
//! command plus the task ids it reported, and "later" is a higher ordinal or
//! a later start.
use crate::{WorkflowV2CallRecord, WorkflowV2HostMethod};

/// `("acceptance-author", Some(3))` for `acceptance-author-3`; the whole id
/// and `None` when it carries no trailing ordinal.
pub fn call_family(call_id: &str) -> (&str, Option<u64>) {
    match call_id.rsplit_once('-') {
        Some((family, ordinal))
            if !ordinal.is_empty() && ordinal.bytes().all(|b| b.is_ascii_digit()) =>
        {
            (family, ordinal.parse().ok())
        }
        _ => (call_id, None),
    }
}

/// The recorded call is history and arrives with the input it was recorded
/// with: replay it verbatim.
pub fn replayable_history(
    record: &WorkflowV2CallRecord,
    records: &[WorkflowV2CallRecord],
    input_hash: &str,
) -> bool {
    record.invalidated_by.is_none()
        && record.input_hash == input_hash
        && superseded(record, records)
}

/// Whether a later, non-invalidated record of the same subject exists.
pub fn superseded(record: &WorkflowV2CallRecord, records: &[WorkflowV2CallRecord]) -> bool {
    match record.call.method {
        WorkflowV2HostMethod::Agent => superseded_agent_record(record, records),
        WorkflowV2HostMethod::HostCommand => superseded_host_record(record, records),
        _ => false,
    }
}

/// An agent record is superseded when a later-ordinal record of the same
/// call family exists.
pub fn superseded_agent_record(
    record: &WorkflowV2CallRecord,
    records: &[WorkflowV2CallRecord],
) -> bool {
    let (family, Some(ordinal)) = call_family(&record.call.id) else {
        return false;
    };
    records.iter().any(|other| {
        other.call.method == WorkflowV2HostMethod::Agent
            && other.invalidated_by.is_none()
            && other.call.id != record.call.id
            && matches!(call_family(&other.call.id), (f, Some(o)) if f == family && o > ordinal)
    })
}

/// A host-command record is superseded when a later-started record of the
/// same command over the same reported task ids exists. A record that reported
/// no subject (a refused candidate never stages one) is keyed by the command
/// alone; it carries no receipt, so replaying it verbatim can never stand in
/// for the artifact on disk.
pub fn superseded_host_record(
    record: &WorkflowV2CallRecord,
    records: &[WorkflowV2CallRecord],
) -> bool {
    let Some(key) = host_subject_key(record) else {
        return false;
    };
    // A record that landed nothing cannot retire one that did: the landed
    // record is what the artifact on disk may still be, and it keeps its
    // live checks until a later landing replaces it.
    let landed = |r: &WorkflowV2CallRecord| !r.result.data["publicationReceipt"].is_null();
    records.iter().any(|other| {
        other.call.method == WorkflowV2HostMethod::HostCommand
            && other.invalidated_by.is_none()
            && other.call.id != record.call.id
            && host_subject_key(other).as_ref() == Some(&key)
            && other.started_at > record.started_at
            && (landed(other) || !landed(record))
    })
}

fn host_subject_key(record: &WorkflowV2CallRecord) -> Option<(String, Vec<String>)> {
    if record.call.method != WorkflowV2HostMethod::HostCommand {
        return None;
    }
    let command = record
        .call
        .options
        .host_command
        .as_ref()?
        .command_id
        .clone();
    let mut tasks: Vec<String> = record.result.data["subjects"]
        .as_array()
        .map(|subjects| {
            subjects
                .iter()
                .filter_map(|subject| subject["taskId"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    tasks.sort();
    Some((command, tasks))
}

#[cfg(test)]
#[path = "history_replay_tests.rs"]
mod tests;
