//! The HIGH gaps no earlier residual pass could plan, owed to the bounded
//! third pass (Issue-121 follow-ups).
//!
//! Since Issue-121 a HIGH gap from any verifier weighs at the final gate.
//! Three kinds had no pass that could plan them, so the gate blocked with
//! nothing able to fix them -- a dead end:
//!
//! - a HIGH gap a first-pass round's verifier recorded while REFUSING (the
//!   second pass plans only an accepted first-pass verifier's new gaps);
//! - a HIGH gap the first or second pass REPORTED instead of planning (the
//!   second pass has no adjudication and a round cap; the third pass left
//!   everything an earlier pass reported alone);
//! - a red test an ACCEPTED remediation verifier's host baseline routed to
//!   the task that declares its file (`write::test_baseline`): the verifier
//!   was excused over it as another task's, the owner never saw it -- its
//!   findings queue is read only by the review reducer, long before -- and
//!   nothing weighed it at all.
//!
//! Each is owed to the third pass unless a round of an earlier pass carried
//! it or the host's own later runs answer it (`residual_superseded`: a
//! refused recorder's gap by its red tests passing by id; a routed test by
//! its own latest run naming it passed). The evidence is bounded exactly as
//! the third pass bounds its own (`cut`: nothing that started after its
//! first round), so asking again while its rounds run owes the same gaps. A
//! routed test is owed ONCE per (test, owner), recorded by the earliest
//! verifier that routed it, as a HIGH gap on its file: the owner is the
//! task accountable, and `route` sends it to the owner's round.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use super::super::WorkflowV2CallRecord;
use super::dispositions::same_gap;
use super::second_pass::residual_key;
use super::superseded::{HostRuns, started};
use super::{
    PlannedRound, Residual, ResidualPlan, ResidualSeverity, SUMMARY_CHARS, accepted_verdict, clip,
    finished, residuals_of,
};
use crate::v2::verification::baseline_run_base::is_unowned_red_gap_id;

/// Id prefix of the gap built from a routed red test (`@` and the test id
/// follow it).
pub const ROUTED_RED_GAP_ID: &str = "baseline_routed_red";

fn is_verify_agent(record: &WorkflowV2CallRecord) -> bool {
    record.invalidated_by.is_none()
        && super::super::remediation_contract_string(&record.call, "stage") == Some("verify")
        && record.call.method != super::super::WorkflowV2HostMethod::Checkpoint
}

fn round_keys(plan: &ResidualPlan) -> BTreeSet<&str> {
    plan.rounds.iter().map(|round| round.key.as_str()).collect()
}

/// The gaps the third pass owes beyond its own population, sorted by key.
pub(super) fn owed_gaps(
    records: &[&WorkflowV2CallRecord],
    stored: &[WorkflowV2CallRecord],
    first: &ResidualPlan,
    second: &ResidualPlan,
    host: &HostRuns,
    cut: Option<i64>,
    root: &Path,
) -> Vec<Residual> {
    let carried: Vec<&Residual> = first
        .rounds
        .iter()
        .chain(&second.rounds)
        .flat_map(|round| &round.residuals)
        .collect();
    let first_keys = round_keys(first);
    let before_cut = |record: &WorkflowV2CallRecord| cut.is_none_or(|at| started(record) < at);
    let mut owed: Vec<Residual> = Vec::new();
    // A first-pass round's verifier that refused.
    for record in stored.iter().filter(|record| {
        is_verify_agent(record)
            && !accepted_verdict(record)
            && residual_key(&record.call).is_some_and(|key| first_keys.contains(key))
            && before_cut(record)
    }) {
        for residual in residuals_of(record, Some(root)) {
            if residual.severity == ResidualSeverity::High
                && !is_unowned_red_gap_id(&residual.id)
                && !carried
                    .iter()
                    .any(|original| same_gap(original, &residual.id, &residual.description))
                && host.superseded_by(&residual, record, cut).is_none()
            {
                owed.push(residual);
            }
        }
    }
    // What an earlier pass reported: a refused recorder's gap its later
    // runs answer is not owed.
    let recorder = |id: &str| stored.iter().find(|record| record.call.id == id);
    for residual in first
        .reported
        .iter()
        .chain(&second.reported)
        .map(|(residual, _)| residual)
        .filter(|residual| residual.severity == ResidualSeverity::High)
    {
        let answered = recorder(&residual.recorded_by).is_some_and(|record| {
            !accepted_verdict(record) && host.superseded_by(residual, record, cut).is_some()
        });
        if !answered {
            owed.push(residual.clone());
        }
    }
    let later_keys: BTreeSet<&str> = first_keys.union(&round_keys(second)).copied().collect();
    owed.extend(routed_gaps(
        records,
        stored,
        &later_keys,
        host,
        cut,
        Some(root),
    ));
    owed.sort_by_key(Residual::key);
    owed.dedup_by_key(|residual| residual.key());
    owed
}

/// Every red test an accepted remediation verifier's host baseline routed to
/// another task, not answered since, once per (test, owner). The recorders
/// are `records` and the stored verifiers of the rounds keyed `rounds` (a
/// resume that skipped a round never replays them), started before `cut`.
pub(super) fn routed_gaps(
    records: &[&WorkflowV2CallRecord],
    stored: &[WorkflowV2CallRecord],
    rounds: &BTreeSet<&str>,
    host: &HostRuns,
    cut: Option<i64>,
    root: Option<&Path>,
) -> Vec<Residual> {
    let mut recorders: Vec<&WorkflowV2CallRecord> = records.to_vec();
    for record in stored {
        if residual_key(&record.call).is_some_and(|key| rounds.contains(key))
            && !recorders.iter().any(|seen| seen.call.id == record.call.id)
        {
            recorders.push(record);
        }
    }
    // Before `cut` only: a third-pass round's own verifier never moves the
    // pass that planned it (its records all start after the cut).
    recorders
        .retain(|record| accepted_verdict(record) && cut.is_none_or(|at| started(record) < at));
    recorders.sort_by(|a, b| started(a).cmp(&started(b)).then(a.call.id.cmp(&b.call.id)));
    // (test, owner) -> the earliest recorder, the file, its commands.
    let mut first: BTreeMap<(String, String), (&WorkflowV2CallRecord, String, BTreeSet<String>)> =
        BTreeMap::new();
    for record in recorders {
        for routed in host.routed(&record.call.id) {
            let entry = first
                .entry((routed.test_id.clone(), routed.owner_task.clone()))
                .or_insert_with(|| (record, routed.file.clone(), BTreeSet::new()));
            if entry.0.call.id == record.call.id {
                entry.2.insert(routed.command.clone());
            }
        }
    }
    let mut owed = Vec::new();
    for ((test, owner), (record, file, commands)) in first {
        let since = finished(record);
        if commands
            .iter()
            .all(|command| host.test_answered(&test, command, since, cut))
        {
            continue;
        }
        let listed: Vec<&str> = commands.iter().map(String::as_str).collect();
        owed.push(Residual {
            recorded_by: record.call.id.clone(),
            id: format!("{ROUTED_RED_GAP_ID}@{test}"),
            severity: ResidualSeverity::High,
            description: format!(
                "`{test}` is red ({}) in {file}, which {owner} declares; the accepted verifier `{}` was excused over it as another task's test, and no round of {owner}'s answered it",
                listed.join("; "),
                record.call.id
            ),
            files: if root.is_some_and(|root| root.join(&file).is_file()) {
                vec![file]
            } else {
                Vec::new()
            },
            unit_tasks: std::iter::once(owner).collect(),
            recorded_summary: clip(&record.result.summary, SUMMARY_CHARS),
        });
    }
    owed
}

/// The third pass's rounds for `owed`, routed as its own gaps are, and keyed
/// apart from them (their gaps are never the same), file rounds first.
pub(super) fn owed_rounds(
    owed: Vec<Residual>,
    route: impl Fn(&Residual) -> Result<(BTreeSet<String>, BTreeSet<String>), String>,
    ids: &BTreeSet<String>,
    plan: &mut ResidualPlan,
) -> Vec<PlannedRound> {
    let mut groups: BTreeMap<Vec<String>, Vec<(Residual, BTreeSet<String>)>> = BTreeMap::new();
    let mut adjudicate: BTreeMap<Vec<String>, Vec<Residual>> = BTreeMap::new();
    for mut residual in owed {
        residual.unit_tasks.retain(|task| ids.contains(task));
        match route(&residual) {
            Ok((tasks, files)) => groups
                .entry(tasks.into_iter().collect())
                .or_default()
                .push((residual, files)),
            Err(_) if ids.contains(&residual.id) => groups
                .entry(vec![residual.id.clone()])
                .or_default()
                .push((residual, BTreeSet::new())),
            Err(_) if !residual.unit_tasks.is_empty() => adjudicate
                .entry(residual.unit_tasks.iter().cloned().collect())
                .or_default()
                .push(residual),
            Err(why) => plan.reported.push((residual, why)),
        }
    }
    let mut rounds = super::third_pass::grouped_rounds(groups, adjudicate);
    // Every one of these carries a HIGH gap: none is ever a medium round.
    debug_assert!(
        rounds
            .iter()
            .all(|round| round.severity() == ResidualSeverity::High)
    );
    rounds.dedup_by(|a, b| a.key == b.key);
    rounds
}

#[cfg(test)]
#[path = "residual_owed_tests.rs"]
mod tests;
