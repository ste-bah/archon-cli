//! Issue-118: the host's bounded SECOND residual pass.
//!
//! The first pass (`residual-gaps-1`) plans rounds for the gaps accepted
//! verifiers recorded before acceptance, once. What those rounds' own
//! verifiers found had nowhere to go: live on wf-0ddadd81 round 3's verifier
//! named seven library tests red since before the run, in files no task and
//! no round may write, and the host refused its verdict over them -- a gap no
//! round could ever be planned for. The second slot (`residual-gaps-2`,
//! [`RESIDUAL_PASS_KEY`] = 2) plans rounds for exactly three kinds of gap,
//! read from the session's verifier records and, from the store, the first
//! pass's own rounds' verifiers:
//!
//! - the host's own record of excused red tests
//!   (`verification::baseline_run_base`), on any verifier, whatever its
//!   verdict: tests that are red and owed, with the files they implicate;
//! - a HIGH gap a first-pass round's accepted verifier recorded that is not
//!   one of that round's own gaps (those the final gate already judges);
//! - the red tests the host refused a first-pass round's verdict over, when
//!   their files lie outside that round's writable scope: the test file and
//!   its parent module file, both derived from the host's own module tree.
//!
//! Anything the first pass already carried or reported is left to it. The
//! gaps are routed exactly as the first pass routes its own (owned or
//! expansion rounds, `residual_plan::route`), under keys of their own, and
//! at most [`MAX_SECOND_PASS_ROUNDS`] rounds are planned; the rest are
//! reported. A gap recorded after this slot is weighed at the final gate;
//! only the HIGH gaps this pass's own rounds' verifiers record are planned
//! again, by the bounded third and final pass (Issue-121,
//! `residual_third_pass`), so the passes cannot loop. A second-pass round's
//! contract carries `residual.pass = 2`, and neither its own records nor a
//! third-pass round's are ever part of this population, so asking again
//! while either pass's rounds run plans the same rounds.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde_json::Value;

use super::super::residual_paths::TaskTexts;
use super::super::{
    WorkflowV2CallRecord, WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2ResultStore,
    remediation_contract, remediation_contract_string,
};
use super::dispositions::same_gap;
use super::{
    MAX_GAPS_PER_ROUND, PlannedRound, RESIDUAL_CONTRACT_KEY, RESIDUAL_GAPS_MARKER, Residual,
    ResidualPlan, ResidualSeverity, RoundKind, accepted_verdict, plan_from, residuals_of, round,
    route,
};
use crate::task_universe::WorkflowV2TaskUniverse;
use crate::v2::verification::baseline_run_base::is_unowned_red_gap_id;

/// The checkpoint option naming which residual pass a slot asks for.
pub const RESIDUAL_PASS_KEY: &str = "residualPass";
/// Most rounds the second pass plans; the rest are reported.
pub const MAX_SECOND_PASS_ROUNDS: usize = 4;
/// Id prefix of the gap the second pass builds from a refused round's red
/// tests (`@` and the refused call's id follow it).
pub const REFUSED_RED_GAP_ID: &str = "baseline_red_outside_scope";

/// Whether `call` is the second pass's slot.
pub fn is_second_pass_slot(call: &WorkflowV2HostCall) -> bool {
    call.method == WorkflowV2HostMethod::Checkpoint
        && call.options.extra.get(RESIDUAL_GAPS_MARKER) == Some(&Value::Bool(true))
        && call
            .options
            .extra
            .get(RESIDUAL_PASS_KEY)
            .and_then(Value::as_u64)
            == Some(2)
}

/// Whether `call` belongs to a round the second pass planned.
pub fn is_second_pass_round(call: &WorkflowV2HostCall) -> bool {
    remediation_contract(call)
        .and_then(|contract| contract.get(RESIDUAL_CONTRACT_KEY))
        .and_then(|claimed| claimed.get("pass"))
        .and_then(Value::as_u64)
        == Some(2)
}

pub(super) fn residual_key(call: &WorkflowV2HostCall) -> Option<&str> {
    remediation_contract(call)
        .and_then(|contract| contract.get(RESIDUAL_CONTRACT_KEY))
        .and_then(|claimed| claimed.get("key"))
        .and_then(Value::as_str)
}

fn is_verify_agent(record: &WorkflowV2CallRecord) -> bool {
    record.invalidated_by.is_none()
        && remediation_contract_string(&record.call, "stage") == Some("verify")
        && record.call.method != WorkflowV2HostMethod::Checkpoint
}

fn is_host_red_gap(residual: &Residual) -> bool {
    is_unowned_red_gap_id(&residual.id)
}

/// The second pass's plan over `records` (the session's records at the
/// slot, or the executed calls before it at the final gate).
pub fn second_pass_plan(
    records: &[&WorkflowV2CallRecord],
    store: &WorkflowV2ResultStore,
    universe: Option<&WorkflowV2TaskUniverse>,
    root: Option<&Path>,
) -> ResidualPlan {
    let (Some(universe), Some(root)) = (universe, root) else {
        return ResidualPlan::default();
    };
    let first = plan_from(records, Some(universe), Some(root));
    let known: BTreeSet<String> = first
        .rounds
        .iter()
        .flat_map(|round| round.residuals.iter().map(Residual::key))
        .chain(first.reported.iter().map(|(residual, _)| residual.key()))
        .collect();
    let rounds: BTreeMap<&str, &PlannedRound> = first
        .rounds
        .iter()
        .map(|round| (round.key.as_str(), round))
        .collect();
    let mut population: Vec<&WorkflowV2CallRecord> = records
        .iter()
        .copied()
        .filter(|record| {
            is_verify_agent(record)
                && !is_second_pass_round(&record.call)
                && !super::is_third_pass_round(&record.call)
                && !super::view::confirm::is_confirmation(&record.call)
        })
        .collect();
    let stored = store.load_call_records().unwrap_or_default();
    for record in stored.iter().filter(|record| {
        is_verify_agent(record)
            && !super::view::confirm::is_confirmation(&record.call)
            && residual_key(&record.call).is_some_and(|k| rounds.contains_key(k))
    }) {
        if !population.iter().any(|seen| seen.call.id == record.call.id) {
            population.push(record);
        }
    }
    // Each first-pass round's own judge: its latest verifier agent.
    let mut judges: BTreeMap<&str, &WorkflowV2CallRecord> = BTreeMap::new();
    for record in &population {
        if let Some(key) = residual_key(&record.call).filter(|k| rounds.contains_key(k)) {
            let entry = judges.entry(key).or_insert(record);
            if super::finished(record) >= super::finished(entry) {
                *entry = record;
            }
        }
    }
    let ids: BTreeSet<String> = universe
        .tasks
        .iter()
        .map(|task| task.canonical_task_id.clone())
        .collect();
    let mut residuals: Vec<Residual> = Vec::new();
    for record in &population {
        let gaps = residuals_of(record, Some(root));
        residuals.extend(gaps.iter().filter(|r| is_host_red_gap(r)).cloned());
        let own = residual_key(&record.call).and_then(|key| rounds.get(key).copied());
        if let Some(own) = own.filter(|_| accepted_verdict(record)) {
            residuals.extend(gaps.into_iter().filter(|r| {
                r.severity == ResidualSeverity::High
                    && !is_host_red_gap(r)
                    && !own
                        .residuals
                        .iter()
                        .any(|original| same_gap(original, &r.id, &r.description))
            }));
        }
    }
    residuals.retain(|residual| !known.contains(&residual.key()));
    residuals.sort_by_key(Residual::key);
    residuals.dedup_by_key(|residual| residual.key());
    let texts = TaskTexts::read(universe, root);
    let mut plan = ResidualPlan::default();
    let mut planned: Vec<PlannedRound> = Vec::new();
    // A first-pass round its judge refused over red tests it could not
    // write is planned AGAIN, whole: its own gaps and those tests, granted
    // their files, judged by one verifier.
    for (key, judge) in &judges {
        let Some(own) = rounds.get(key) else {
            continue;
        };
        if let Some(red) = retry::refused_red_tests(store, judge, own, universe, root) {
            match retry::retry_round(own, red, universe, root) {
                Ok(retry) => planned.push(retry),
                Err((red, why)) => plan.reported.push((red, why)),
            }
        }
    }
    let mut groups: BTreeMap<Vec<String>, Vec<(Residual, BTreeSet<String>)>> = BTreeMap::new();
    for mut residual in residuals {
        residual.unit_tasks.retain(|task| ids.contains(task));
        match route(&residual, universe, root, &texts) {
            Ok((tasks, files)) => groups
                .entry(tasks.into_iter().collect())
                .or_default()
                .push((residual, files)),
            Err(why) => plan.reported.push((residual, why)),
        }
    }
    for (tasks, members) in groups {
        for chunk in members.chunks(MAX_GAPS_PER_ROUND) {
            let files: BTreeSet<String> = chunk.iter().flat_map(|(_, f)| f.clone()).collect();
            let kind = if files.is_empty() {
                RoundKind::Owned
            } else {
                RoundKind::Expansion
            };
            let residuals: Vec<Residual> = chunk.iter().map(|(r, _)| r.clone()).collect();
            planned.push(second_key(round(
                kind,
                tasks.clone(),
                files,
                residuals,
                None,
                None,
            )));
        }
    }
    planned.sort_by(|a, b| b.severity().cmp(&a.severity()).then(a.key.cmp(&b.key)));
    let why = format!("the second residual pass plans at most {MAX_SECOND_PASS_ROUNDS} rounds");
    for (at, planned_round) in planned.into_iter().enumerate() {
        if at < MAX_SECOND_PASS_ROUNDS {
            plan.rounds.push(planned_round);
        } else {
            plan.reported.extend(
                planned_round
                    .residuals
                    .into_iter()
                    .map(|r| (r, why.clone())),
            );
        }
    }
    plan
}

/// A key of the second pass's own: never one the first could have planned.
pub(super) fn second_key(mut planned: PlannedRound) -> PlannedRound {
    planned.key = format!("{}p2", planned.key);
    planned.pass = 2;
    planned
}

#[path = "residual_second_pass_retry.rs"]
mod retry;
