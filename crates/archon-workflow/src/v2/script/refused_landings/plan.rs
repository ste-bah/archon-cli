//! Which landings of the run's remediation fixes their unit refused.
//!
//! Every record of a remediation names its unit and round in its contract
//! (`resume_drift::remediation_unit`). Only units the acceptance stage
//! routed are in scope. A unit's fix records are the calls that land; its
//! verdicts are the agent verifiers that judged the tree they left. A call a
//! pause interrupted, and a no-patch checkpoint, judged nothing and are no
//! verdict: a landing no verdict judged is PENDING and never reverted.
//!
//! A landing of fix stage S is attributed to the unit of S's record and is
//! placed in time: a data line by the time it was logged, a copy by the time
//! it was placed, a commit by its commit second, all bounded by its fix's
//! record; a copy logged before its time was recorded, and a serial write,
//! by its fix's round instead (judged by every verdict of that round on). The verdicts that finished after it judged a tree holding
//! it. It STANDS when one of them accepted -- an agent verdict whose round's
//! fix was accepted too -- and is REFUSED when at least one judged it and
//! none accepted; with no verdict after it, it is PENDING. Records are
//! always read as they are now: a verdict a later session overwrote is its
//! latest answer.

use std::collections::BTreeMap;

use serde_json::Value;

use super::super::resume_drift::remediation_unit;
use super::super::resume_freshness::OBSERVED_BY_KEY;
use super::super::{
    WorkflowV2CallRecord, WorkflowV2HostMethod, WorkflowV2Status, WorkflowV2WriteMode,
    is_reusable_status, is_transport_failure_text, remediation_contract,
};

/// A remediation record's place in its unit.
pub(super) struct Judged<'a> {
    pub(super) record: &'a WorkflowV2CallRecord,
    pub(super) at: i64,
    /// The round its contract names.
    pub(super) round: u64,
}

/// One unit of remediation: its fixes and its verdicts, oldest first.
pub(super) struct Unit<'a> {
    pub(super) task_ids: Vec<String>,
    pub(super) fixes: Vec<Judged<'a>>,
    pub(super) verdicts: Vec<Judged<'a>>,
}

/// Where a landing stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Standing {
    Stands,
    Pending,
    /// Refused; the call id and summary of the latest verdict that judged it.
    Refused {
        verdict: String,
        summary: String,
    },
}

/// `finished_at` in nanoseconds since the epoch; `None` when unreadable.
pub(super) fn finished(record: &WorkflowV2CallRecord) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(&record.finished_at)
        .ok()
        .and_then(|at| at.timestamp_nanos_opt())
}

fn interrupted(record: &WorkflowV2CallRecord) -> bool {
    record
        .result
        .data
        .get("interrupted")
        .is_some_and(Value::is_string)
}

fn stage(record: &WorkflowV2CallRecord) -> Option<&str> {
    remediation_contract(&record.call)?.get("stage")?.as_str()
}

/// The tasks a unit speaks for: its contract's `taskIds`, else `taskId`,
/// and an escalated round's owners.
fn contract_tasks(record: &WorkflowV2CallRecord) -> Vec<String> {
    let Some(contract) = remediation_contract(&record.call) else {
        return Vec::new();
    };
    let strings = |value: Option<&Value>| -> Vec<String> {
        (value.and_then(Value::as_array).into_iter().flatten())
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect()
    };
    let mut tasks = strings(contract.get("taskIds"));
    if tasks.is_empty()
        && let Some(task) = contract.get("taskId").and_then(Value::as_str)
    {
        tasks.push(task.to_string());
    }
    tasks.extend(strings(contract.pointer("/escalation/ownerTaskIds")));
    tasks
}

/// Every remediation unit among `records`, keyed by unit.
pub(super) fn units(records: &[WorkflowV2CallRecord]) -> BTreeMap<String, Unit<'_>> {
    let mut units: BTreeMap<String, Unit<'_>> = BTreeMap::new();
    for record in records {
        let Some((key, round)) = remediation_unit(&record.call) else {
            continue;
        };
        let Some(at) = finished(record) else {
            continue;
        };
        // Batch L3: only units the acceptance stage routed (their contract
        // names the observation they fix) are in scope; review, residual and
        // contest units are earlier history. An interrupted call and a
        // no-patch checkpoint judged nothing: they are no verdict.
        let routed = remediation_contract(&record.call)
            .is_some_and(|contract| contract.get(OBSERVED_BY_KEY).is_some());
        // Nor is a verify the transport cut off or a stop cancelled.
        let unjudged = stage(record) == Some("verify")
            && (record.status == WorkflowV2Status::Cancelled
                || is_transport_failure_text(&record.result.summary));
        if !routed
            || unjudged
            || interrupted(record)
            || record.call.method == WorkflowV2HostMethod::Checkpoint
        {
            continue;
        }
        let unit = units.entry(key).or_insert_with(|| Unit {
            task_ids: Vec::new(),
            fixes: Vec::new(),
            verdicts: Vec::new(),
        });
        for task in contract_tasks(record) {
            if !unit.task_ids.contains(&task) {
                unit.task_ids.push(task);
            }
        }
        match stage(record) {
            Some("remediate") if record.call.write_mode.is_some() => {
                unit.fixes.push(Judged { record, at, round });
            }
            Some("verify") => unit.verdicts.push(Judged { record, at, round }),
            _ => {}
        }
    }
    for unit in units.values_mut() {
        unit.fixes.sort_by_key(|fix| fix.at);
        unit.verdicts.sort_by_key(|verdict| verdict.at);
    }
    units
}

impl Unit<'_> {
    /// Whether `verdict` accepted: an agent verdict, accepted, judging a
    /// round whose fix was accepted too.
    fn accepted(&self, verdict: &Judged<'_>) -> bool {
        if verdict.record.call.method == WorkflowV2HostMethod::Checkpoint
            || !is_reusable_status(verdict.record.status)
        {
            return false;
        }
        // Paired by the round both contracts name, never by time: a fix's
        // record is written again when a later session replays it.
        self.fixes
            .iter()
            .any(|fix| fix.round == verdict.round && is_reusable_status(fix.record.status))
    }

    /// Whether any landing of this unit can be refused or unjudged: not
    /// when its latest round's fix and verdict both accepted, since that
    /// verdict judged every landing before it and a fix is followed by its
    /// verdict.
    pub(super) fn may_refuse(&self) -> bool {
        self.verdicts
            .last()
            .is_some_and(|verdict| !self.accepted(verdict))
    }

    /// Where a landing of this unit made at `at` stands.
    pub(super) fn standing(&self, place: Place) -> Standing {
        let after: Vec<&Judged<'_>> = (self.verdicts.iter())
            .filter(|verdict| match place {
                Place::At(at) => verdict.at > at,
                Place::Round(round) => verdict.round >= round,
            })
            .collect();
        if after.iter().any(|verdict| self.accepted(verdict)) {
            return Standing::Stands;
        }
        match after.last() {
            None => Standing::Pending,
            Some(verdict) => Standing::Refused {
                verdict: verdict.record.call.id.clone(),
                summary: verdict.record.result.summary.chars().take(1200).collect(),
            },
        }
    }

    /// Fixes of this unit that wrote in serial mode, whose writes no landing
    /// recorded and no revert can find, and that a verdict refused.
    pub(super) fn refused_serial_fixes(&self) -> Vec<(&Judged<'_>, Standing)> {
        self.fixes
            .iter()
            .filter(|fix| fix.record.call.write_mode == Some(WorkflowV2WriteMode::Serial))
            .filter(|fix| {
                !super::super::remediation_escalation::landed_nothing(&fix.record.result.data)
            })
            .filter_map(|fix| match self.standing(Place::Round(fix.round)) {
                refused @ Standing::Refused { .. } => Some((fix, refused)),
                _ => None,
            })
            .collect()
    }
}

/// Where a landing sits among its unit's verdicts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Place {
    /// At this time: judged by every verdict that finished after it.
    At(i64),
    /// In this round, its time unknown: judged by every verdict of this
    /// round or a later one. A record's own time cannot place it: a later
    /// session writes a replayed fix's record again.
    Round(u64),
}

/// Where a landing of `fix` logged at `logged` sits: at that time, never
/// after the fix's own record (written once it had landed), or in its fix's
/// round when the time was not logged.
pub(super) fn landing_place(fix: &Judged<'_>, logged: Option<i64>) -> Place {
    match logged.filter(|at| *at > 0) {
        Some(at) => Place::At(at.min(fix.at.saturating_sub(1))),
        None => Place::Round(fix.round),
    }
}
