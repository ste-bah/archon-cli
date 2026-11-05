//! Journal records used to derive phase seeds.

use super::*;

/// A judged gate: its record, command, stdin and what the script saw.
pub(super) type Gate<'a> = (&'a WorkflowV2CallRecord, &'a str, &'a str, GateSeed);

/// An acceptance author reply the script can read: its entry, with the
/// host-owned fields of an owed check set, and its call ordinal.
pub(super) struct Reply<'a> {
    pub(super) record: &'a WorkflowV2CallRecord,
    pub(super) id: String,
    pub(super) ordinal: u64,
    pub(super) text: String,
    pub(super) entry: Value,
}

