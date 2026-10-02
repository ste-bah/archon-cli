//! Issue-121: the host's bounded THIRD and final residual pass.
//!
//! The second pass (`residual-gaps-2`) plans rounds for what the first
//! pass's rounds left; what ITS rounds' verifiers found had nowhere to go.
//! Live on wf-0ddadd81 a second-pass round's verifier refused and recorded,
//! as HIGH, a regression an earlier round landed in another task's file:
//! three declared must-pass tests red, recorded after the second pass had
//! planned, so no round could target it and the final gate did not even
//! weigh it (a refused verdict's gaps were read only for the host's own red
//! tests).
//!
//! The third slot (`residual-gaps-3`, [`super::RESIDUAL_PASS_KEY`] = 3)
//! plans rounds for the gaps -- at ANY severity since Batch O -- a
//! second-pass round's verifier agent recorded, whatever its verdict, that
//! are not exactly a gap the first two passes carried (`same_identity`: an
//! ambiguous match is a new gap), and -- for a verifier that refused --
//! that the host's own later test runs do not answer
//! (`residual_superseded`). They are routed exactly as the first pass routes
//! its own (`route`: owned or expansion rounds, an adjudication for a gap no
//! file round can carry) -- except that the host's own restatement of a
//! refused task, which names no file, joins that task's round -- file
//! rounds first, and EVERY round is planned (Batch O: no round cap). Each
//! second-pass file round its own judge left open is planned again, whole
//! (`retry::again`), after those.
//! A gap recorded after this slot is planned by the fourth pass only while
//! the passes make progress (Batch O2, `residual_later_pass`); otherwise it
//! is weighed at the final gate, where it blocks. Since Issue-121's
//! follow-ups the pass also plans, in rounds of their own after those above, the gaps no earlier
//! pass could (`residual_owed`: a refused first-pass verifier's, one an
//! earlier pass reported, a red test an accepted verifier's baseline routed
//! to its file's owner). A third-pass round's contract carries
//! `residual.pass = 3`, and no pass's population ever includes its records,
//! so asking again while its rounds run plans the same rounds, and the first
//! two passes' plans never move.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde_json::Value;

use super::super::residual_paths::TaskTexts;
use super::super::{
    WorkflowV2CallRecord, WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2ResultStore,
    remediation_contract,
};
use super::dispositions::same_identity;
use super::second_pass::residual_key;
use super::superseded::HostRuns;
use super::{
    MAX_GAPS_PER_ROUND, PlannedRound, RESIDUAL_CONTRACT_KEY, RESIDUAL_GAPS_MARKER,
    RESIDUAL_PASS_KEY, Residual, ResidualPlan, RoundKind, accepted_verdict, plan_from,
    residuals_of, round, route, second_pass_plan,
};
use crate::task_universe::WorkflowV2TaskUniverse;

/// Whether `call` is the third pass's slot.
pub fn is_third_pass_slot(call: &WorkflowV2HostCall) -> bool {
    call.method == WorkflowV2HostMethod::Checkpoint
        && call.options.extra.get(RESIDUAL_GAPS_MARKER) == Some(&Value::Bool(true))
        && call
            .options
            .extra
            .get(RESIDUAL_PASS_KEY)
            .and_then(Value::as_u64)
            == Some(3)
}

/// Whether `call` belongs to a round the third pass planned.
pub fn is_third_pass_round(call: &WorkflowV2HostCall) -> bool {
    remediation_contract(call)
        .and_then(|contract| contract.get(RESIDUAL_CONTRACT_KEY))
        .and_then(|claimed| claimed.get("pass"))
        .and_then(Value::as_u64)
        == Some(3)
}

/// The third pass's plan over `records` (the session's records at the slot,
/// or the executed calls before it at the final gate).
pub fn third_pass_plan(
    records: &[&WorkflowV2CallRecord],
    store: &WorkflowV2ResultStore,
    universe: Option<&WorkflowV2TaskUniverse>,
    root: Option<&Path>,
) -> ResidualPlan {
    let (Some(universe), Some(root)) = (universe, root) else {
        return ResidualPlan::default();
    };
    let first = plan_from(records, Some(universe), Some(root));
    let second = second_pass_plan(records, store, Some(universe), Some(root));
    // Each gap a round of the first two passes carried, with the tasks of
    // the round that carried it: its identity (`same_identity`).
    let carried: Vec<(&BTreeSet<String>, &Residual)> = first
        .rounds
        .iter()
        .chain(&second.rounds)
        .flat_map(|round| round.residuals.iter().map(move |r| (&round.tasks, r)))
        .collect();
    let known: BTreeSet<String> = carried
        .iter()
        .map(|(_, residual)| residual.key())
        .chain(first.reported.iter().map(|(residual, _)| residual.key()))
        .chain(second.reported.iter().map(|(residual, _)| residual.key()))
        .collect();
    let rounds: BTreeMap<&str, &PlannedRound> = second
        .rounds
        .iter()
        .map(|round| (round.key.as_str(), round))
        .collect();
    let host = HostRuns::load(store);
    let stored = store.load_call_records().unwrap_or_default();
    // Evidence only from before this pass's first round started: once its
    // rounds run, the plan they were asked under never moves.
    let cut = stored
        .iter()
        .filter(|record| is_third_pass_round(&record.call))
        .map(super::superseded::started)
        .min();
    let mut residuals: Vec<Residual> = Vec::new();
    for record in &stored {
        if record.invalidated_by.is_some()
            || super::super::remediation_contract_string(&record.call, "stage") != Some("verify")
            || record.call.method == WorkflowV2HostMethod::Checkpoint
            || super::view::confirm::is_confirmation(&record.call)
            || !residual_key(&record.call).is_some_and(|key| rounds.contains_key(key))
        {
            continue;
        }
        let accepted = accepted_verdict(record);
        for residual in residuals_of(record, Some(root)) {
            if carried
                .iter()
                .any(|(owners, original)| same_identity(original, owners, &residual))
                || (!accepted && host.superseded_by(&residual, record, cut).is_some())
            {
                continue;
            }
            residuals.push(residual);
        }
    }
    residuals.retain(|residual| !known.contains(&residual.key()));
    residuals.sort_by_key(Residual::key);
    residuals.dedup_by_key(|residual| residual.key());
    let ids: BTreeSet<String> = universe
        .tasks
        .iter()
        .map(|task| task.canonical_task_id.clone())
        .collect();
    let texts = TaskTexts::read(universe, root);
    let mut plan = ResidualPlan::default();
    let mut groups: BTreeMap<Vec<String>, Vec<(Residual, BTreeSet<String>)>> = BTreeMap::new();
    let mut adjudicate: BTreeMap<Vec<String>, Vec<Residual>> = BTreeMap::new();
    let own_keys: BTreeSet<String> = residuals.iter().map(Residual::key).collect();
    for mut residual in residuals {
        residual.unit_tasks.retain(|task| ids.contains(task));
        match route(&residual, universe, root, &texts) {
            Ok((tasks, files)) => groups
                .entry(tasks.into_iter().collect())
                .or_default()
                .push((residual, files)),
            // The host's own restatement of a refused task (`record_landing`:
            // the gap's id is the task's) is that task's: it rides the
            // task's file round, judged by the same verifier.
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
    let mut planned = grouped_rounds(groups, adjudicate);
    // Batch O: each second-pass file round its own judge -- its latest
    // verifier agent before the cut -- left open is planned again, whole,
    // with that judgment quoted, after this pass's own rounds.
    let mut judges: BTreeMap<&str, &WorkflowV2CallRecord> = BTreeMap::new();
    for record in stored.iter().filter(|record| {
        record.invalidated_by.is_none()
            && super::super::remediation_contract_string(&record.call, "stage") == Some("verify")
            && record.call.method != WorkflowV2HostMethod::Checkpoint
            && !super::view::confirm::is_confirmation(&record.call)
            && cut.is_none_or(|at| super::superseded::started(record) < at)
    }) {
        if let Some(key) = residual_key(&record.call).filter(|key| rounds.contains_key(key)) {
            let entry = judges.entry(key).or_insert(record);
            if super::finished(record) >= super::finished(entry) {
                *entry = record;
            }
        }
    }
    let mut again: Vec<PlannedRound> = judges
        .iter()
        .filter_map(|(key, judge)| {
            let own = rounds.get(key)?;
            super::second_pass::retry::left_open(judge, own)
                .then(|| third_key(super::second_pass::retry::again(own, judge)))
        })
        .collect();
    again.sort_by(|a, b| a.key.cmp(&b.key));
    let retried: BTreeSet<String> = again
        .iter()
        .flat_map(|round| round.residuals.iter().map(Residual::key))
        .collect();
    planned.extend(again);
    // What no earlier pass could plan (`residual_owed`), in rounds of its
    // own AFTER the ones above: those keep their keys and their places, so a
    // pass whose rounds already ran plans them exactly as it did.
    // Batch E: and every refused project-input landing no later landing
    // answered, as its tasks' own restatement.
    let owed: Vec<Residual> =
        super::owed::owed_gaps(records, &stored, &first, &second, &host, cut, root)
            .into_iter()
            .chain(super::owed::refused_input_gaps(store, cut))
            .filter(|residual| {
                !own_keys.contains(&residual.key()) && !retried.contains(&residual.key())
            })
            .collect();
    planned.extend(super::owed::owed_rounds(
        owed,
        |residual| route(residual, universe, root, &texts),
        &ids,
        &mut plan,
    ));
    // Batch O2: the host's own regressions found before this pass, last.
    let known: BTreeSet<String> = known
        .into_iter()
        .chain(
            planned
                .iter()
                .flat_map(|r| r.residuals.iter().map(Residual::key)),
        )
        .collect();
    let regressions = super::regression::regression_gaps(store, &stored, universe, 3, &known);
    planned.extend(super::regression::regression_rounds(
        regressions,
        universe,
        root,
        third_key,
        &mut plan.reported,
    ));
    // Every round is planned: no cap turns one into a report -- unless the
    // run's pass ceiling is passed (Issue-225), when every gap is reported.
    plan.rounds = planned;
    super::later_pass::capped(3, plan, &second, store, &stored)
}

/// One pass's rounds from its routed groups: file rounds in key order, then
/// adjudications in key order, after every file round has landed.
pub(super) fn grouped_rounds(
    groups: BTreeMap<Vec<String>, Vec<(Residual, BTreeSet<String>)>>,
    adjudicate: BTreeMap<Vec<String>, Vec<Residual>>,
) -> Vec<PlannedRound> {
    let mut planned: Vec<PlannedRound> = Vec::new();
    for (tasks, members) in groups {
        for chunk in members.chunks(MAX_GAPS_PER_ROUND) {
            let files: BTreeSet<String> = chunk.iter().flat_map(|(_, f)| f.clone()).collect();
            let kind = if files.is_empty() {
                RoundKind::Owned
            } else {
                RoundKind::Expansion
            };
            let residuals: Vec<Residual> = chunk.iter().map(|(r, _)| r.clone()).collect();
            planned.push(third_key(round(
                kind,
                tasks.clone(),
                files,
                residuals,
                None,
                None,
            )));
        }
    }
    planned.sort_by(|a, b| a.key.cmp(&b.key));
    let mut adjudications: Vec<PlannedRound> = Vec::new();
    for (tasks, members) in adjudicate {
        for chunk in members.chunks(MAX_GAPS_PER_ROUND) {
            adjudications.push(third_key(round(
                RoundKind::Adjudication,
                tasks.clone(),
                BTreeSet::new(),
                chunk.to_vec(),
                None,
                None,
            )));
        }
    }
    adjudications.sort_by(|a, b| a.key.cmp(&b.key));
    planned.extend(adjudications);
    planned
}

/// A key of the third pass's own: never one the first two could have planned.
fn third_key(mut planned: PlannedRound) -> PlannedRound {
    planned.key = format!("{}p3", planned.key);
    planned.pass = 3;
    planned
}
