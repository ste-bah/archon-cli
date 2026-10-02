//! Batch O2 (REM-11/CUT-5): residual passes that follow progress, not a
//! count of three.
//!
//! Passes 1-3 are what they always were (their slots ask exactly what they
//! always asked, so a resumed run replays them). After the third, the
//! prelude asks the slot of pass N = 4, 5, ... (`residual-gaps-N`,
//! [`super::RESIDUAL_PASS_KEY`] = N) and stops at the first pass the host
//! plans nothing for. Pass N plans, exactly as the third pass plans over the
//! second's rounds, over pass N-1's rounds -- evidence bounded by pass N's
//! own first round (`cut`), so asking again while its rounds run plans the
//! same rounds:
//!
//! - NEW work: every gap a verifier of pass N-1's rounds recorded that no
//!   earlier pass carried or reported (`same_identity`; a refused
//!   verifier's gap the host's own later runs answer is not work), and every
//!   owed gap (`residual_owed`), regression (`residual_regression`) or
//!   refused project-input landing no earlier pass carried;
//! - a RETRY of each pass N-1 file round its judge left open, only when it
//!   made progress: it was a first attempt, or its judge left FEWER of the
//!   round's gaps open than the judge of the attempt it retried.
//!
//! Pass N is planned at all only while the open gaps -- any severity, by id
//! -- move: pass N-1 left a set unlike pass N-2's.
//!
//! A pass that plans nothing ends the passes; what stands then blocks at
//! the final gate. Every retry needs a strictly smaller open count, and a
//! gap once carried is never new again -- but a verifier that records a
//! NEW gap every pass keeps the open set moving forever, so progress alone
//! does not end them. Two stops do (`residual_pass_stops`, Issue-225): an
//! open set an earlier pass already left (a cycle), and the run's pass
//! ceiling. A stopped pass plans no round and reports every open gap.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde_json::Value;

use super::super::residual_paths::TaskTexts;
use super::super::{
    WorkflowV2CallRecord, WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2ResultStore,
    is_reusable_status,
};
use super::dispositions::{Disposition, bare_id, disposition_of, same_gap, same_identity};
use super::second_pass::residual_key;
use super::superseded::{HostRuns, started};
use super::{
    PlannedRound, Residual, ResidualPlan, accepted_verdict, plan_from, residuals_of, route,
    second_pass_plan, third_pass_plan,
};
use crate::task_universe::WorkflowV2TaskUniverse;

/// Which residual pass the slot checkpoint `call` asks for: 1 for the
/// first (it names none), `None` for any other call.
pub fn slot_pass(call: &WorkflowV2HostCall) -> Option<u64> {
    (call.method == WorkflowV2HostMethod::Checkpoint
        && call.options.extra.get(super::RESIDUAL_GAPS_MARKER) == Some(&Value::Bool(true)))
    .then(|| {
        call.options
            .extra
            .get(super::RESIDUAL_PASS_KEY)
            .and_then(Value::as_u64)
            .unwrap_or(1)
    })
}

/// Which residual pass planned the round `call` belongs to: 1 when its
/// contract names none, `None` for a call of no residual round.
pub fn round_pass(call: &WorkflowV2HostCall) -> Option<u64> {
    let claimed = super::super::remediation_contract(call)?.get(super::RESIDUAL_CONTRACT_KEY)?;
    Some(claimed.get("pass").and_then(Value::as_u64).unwrap_or(1))
}

/// Batch O2 (M3): whether the run recorded an acceptance round or a residual
/// confirmation -- what a prelude runs once its residual passes are over --
/// that started before `before` (any, when `None`).
pub fn recording_moved_on(stored: &[WorkflowV2CallRecord], before: Option<i64>) -> bool {
    stored.iter().any(|record| {
        record.invalidated_by.is_none()
            && (super::super::is_acceptance_stage_call(&record.call)
                || super::view::confirm::is_confirmation(&record.call))
            && before.is_none_or(|at| started(record) < at)
    })
}

/// Whether `call` belongs to a round of pass 4 or later.
pub fn is_later_pass_round(call: &WorkflowV2HostCall) -> bool {
    super::round_pass(call).is_some_and(|pass| pass >= 4)
}

/// The plans of passes 1..=`n`, each over `records`.
pub fn pass_plans(
    n: u64,
    records: &[&WorkflowV2CallRecord],
    store: &WorkflowV2ResultStore,
    universe: Option<&WorkflowV2TaskUniverse>,
    root: Option<&Path>,
) -> Vec<ResidualPlan> {
    let mut plans = vec![plan_from(records, universe, root)];
    if n >= 2 {
        plans.push(second_pass_plan(records, store, universe, root));
    }
    if n >= 3 {
        plans.push(third_pass_plan(records, store, universe, root));
    }
    let (Some(universe), Some(root)) = (universe, root) else {
        // Nothing to map a gap through: every later pass plans nothing.
        plans.resize_with(n.max(1) as usize, ResidualPlan::default);
        return plans;
    };
    let stored = store.load_call_records().unwrap_or_default();
    for pass in 4..=n {
        let plan = later(pass, &plans, records, &stored, store, universe, root);
        let plan = stops::checked(pass, plan, &plans, store, &stored);
        plans.push(plan);
    }
    plans
}

/// Pass `n`'s plan (`n` >= 4).
pub fn later_pass_plan(
    n: u64,
    records: &[&WorkflowV2CallRecord],
    store: &WorkflowV2ResultStore,
    universe: Option<&WorkflowV2TaskUniverse>,
    root: Option<&Path>,
) -> ResidualPlan {
    pass_plans(n, records, store, universe, root)
        .pop()
        .unwrap_or_default()
}

fn later(
    n: u64,
    plans: &[ResidualPlan],
    records: &[&WorkflowV2CallRecord],
    stored: &[WorkflowV2CallRecord],
    store: &WorkflowV2ResultStore,
    universe: &WorkflowV2TaskUniverse,
    root: &Path,
) -> ResidualPlan {
    let previous = &plans[plans.len() - 1];
    // Batch O2 (M3): a recording that already moved on -- an acceptance
    // round or a residual confirmation started before this pass's slot was
    // asked (or its first round, whichever is earlier) -- was made by a
    // prelude that asked no such pass; its resume plans nothing here, so
    // nothing it recorded is re-opened.
    let asked = stored
        .iter()
        .filter(|record| {
            super::round_pass(&record.call) == Some(n) || super::slot_pass(&record.call) == Some(n)
        })
        .map(started)
        .min();
    if recording_moved_on(stored, asked) {
        return ResidualPlan::default();
    }
    // Batch O2 (M1): a pass is planned only while the open gaps move -- any
    // severity, by id: pass N-1 left a set unlike pass N-2's. Otherwise every
    // gap still open stands, quoted, and blocks.
    let before = &plans[plans.len() - 2];
    let open_now = open_ids(previous, stored);
    if open_now == open_ids(before, stored) {
        return stalled(n, previous, stored, &open_now);
    }
    let carried: Vec<(&BTreeSet<String>, &Residual)> = plans
        .iter()
        .flat_map(|plan| &plan.rounds)
        .flat_map(|round| round.residuals.iter().map(move |r| (&round.tasks, r)))
        .collect();
    let mut known: BTreeSet<String> = carried
        .iter()
        .map(|(_, residual)| residual.key())
        .chain(
            plans
                .iter()
                .flat_map(|plan| plan.reported.iter().map(|(r, _)| r.key())),
        )
        .collect();
    let rounds: BTreeMap<&str, &PlannedRound> = previous
        .rounds
        .iter()
        .map(|round| (round.key.as_str(), round))
        .collect();
    let host = HostRuns::load(store);
    // Evidence bounded by what the slot itself saw: nothing that started
    // after it was asked, nor after this pass's first round.
    let cut = stored
        .iter()
        .filter(|record| super::round_pass(&record.call) == Some(n))
        .map(started)
        .chain(
            stored
                .iter()
                .filter(|record| super::slot_pass(&record.call) == Some(n))
                .map(super::finished),
        )
        .min();
    let before_cut = |record: &WorkflowV2CallRecord| cut.is_none_or(|at| started(record) < at);
    let judging = |record: &&WorkflowV2CallRecord| {
        record.invalidated_by.is_none()
            && super::super::remediation_contract_string(&record.call, "stage") == Some("verify")
            && record.call.method != WorkflowV2HostMethod::Checkpoint
            && !super::view::confirm::is_confirmation(&record.call)
            && residual_key(&record.call).is_some_and(|key| rounds.contains_key(key))
            && before_cut(record)
    };
    let mut fresh: Vec<Residual> = Vec::new();
    let mut judges: BTreeMap<&str, &WorkflowV2CallRecord> = BTreeMap::new();
    for record in stored.iter().filter(judging) {
        let accepted = accepted_verdict(record);
        for residual in residuals_of(record, Some(root)) {
            if is_new(&residual, &carried)
                && (accepted || host.superseded_by(&residual, record, cut).is_none())
            {
                fresh.push(residual);
            }
        }
        if let Some(key) = residual_key(&record.call) {
            let entry = judges.entry(key).or_insert(record);
            if super::finished(record) >= super::finished(entry) {
                *entry = record;
            }
        }
    }
    fresh.retain(|residual| !known.contains(&residual.key()));
    fresh.sort_by_key(Residual::key);
    fresh.dedup_by_key(|residual| residual.key());
    let rekey = |planned: PlannedRound| rekey(planned, n);
    let ids: BTreeSet<String> = universe
        .tasks
        .iter()
        .map(|task| task.canonical_task_id.clone())
        .collect();
    let texts = TaskTexts::read(universe, root);
    let mut plan = ResidualPlan::default();
    known.extend(fresh.iter().map(Residual::key));
    let mut planned: Vec<PlannedRound> = super::owed::owed_rounds(
        fresh,
        |residual| route(residual, universe, root, &texts),
        &ids,
        &mut plan,
    )
    .into_iter()
    .map(rekey)
    .collect();
    // Retries that made progress, after the new work.
    let mut again: Vec<PlannedRound> = judges
        .iter()
        .filter_map(|(key, judge)| {
            let own = rounds.get(key)?;
            (super::second_pass::retry::left_open(judge, own) && progressed(own, judge, store))
                .then(|| rekey(super::second_pass::retry::again(own, judge)))
        })
        .collect();
    again.sort_by(|a, b| a.key.cmp(&b.key));
    known.extend(
        again
            .iter()
            .flat_map(|r| r.residuals.iter().map(Residual::key)),
    );
    planned.extend(again);
    // What no earlier pass could plan, the refused project-input landings
    // and the host's own regressions, never planned before.
    let owed: Vec<Residual> =
        super::owed::owed_gaps(records, stored, &plans[0], &plans[1], &host, cut, root)
            .into_iter()
            .chain(super::owed::refused_input_gaps(store, cut))
            .filter(|residual| !known.contains(&residual.key()))
            .collect();
    known.extend(owed.iter().map(Residual::key));
    planned.extend(
        super::owed::owed_rounds(owed, |r| route(r, universe, root, &texts), &ids, &mut plan)
            .into_iter()
            .map(rekey),
    );
    let regressions = super::regression::regression_gaps(store, stored, universe, n, &known);
    planned.extend(super::regression::regression_rounds(
        regressions,
        universe,
        root,
        rekey,
        &mut plan.reported,
    ));
    let mut seen = BTreeSet::new();
    planned.retain(|round| seen.insert(round.key.clone()));
    plan.rounds = planned;
    plan
}

/// A round of pass `n`: its own key, never one an earlier pass planned.
fn rekey(mut planned: PlannedRound, n: u64) -> PlannedRound {
    let base = planned
        .key
        .strip_suffix("p3")
        .unwrap_or(&planned.key)
        .to_string();
    planned.key = format!("{base}p{n}");
    planned.pass = u8::try_from(n).unwrap_or(u8::MAX);
    planned
}

/// Whether retrying `own` again is progress: a first attempt (it answers
/// no earlier judgment), or its judge left fewer of its gaps open than the
/// judge of the attempt it retried.
fn progressed(
    own: &PlannedRound,
    judge: &WorkflowV2CallRecord,
    store: &WorkflowV2ResultStore,
) -> bool {
    let Some(earlier) = own
        .refusal
        .as_ref()
        .and_then(|refusal| refusal.get("verifier"))
        .and_then(Value::as_str)
    else {
        return true;
    };
    let Some(earlier) = store.load_call_record(earlier).ok().flatten() else {
        return true;
    };
    open_count(judge, own) < open_count(&earlier, own)
}

/// How many of `round`'s gaps `judge` left open: every one when it refused
/// without saying which, else each it did not dispose `resolved`.
fn open_count(judge: &WorkflowV2CallRecord, round: &PlannedRound) -> usize {
    let said: Vec<Option<Disposition>> = round
        .residuals
        .iter()
        .map(|gap| disposition_of(judge, &gap.id))
        .collect();
    if !is_reusable_status(judge.status) && said.iter().all(Option::is_none) {
        return round.residuals.len();
    }
    said.iter()
        .filter(|said| **said != Some(Disposition::Resolved))
        .count()
}

/// M1: whether `residual` is new work -- no carried gap is it, exactly or
/// loosely (its id, or its opening words): a reworded gap is not new.
fn is_new(residual: &Residual, carried: &[(&BTreeSet<String>, &Residual)]) -> bool {
    !carried.iter().any(|(owners, original)| {
        same_identity(original, owners, residual)
            || same_gap(original, &residual.id, &residual.description)
    })
}

/// Each verifier agent of `round` (never a confirmation), latest last.
fn verifiers<'a>(
    round: &PlannedRound,
    stored: &'a [WorkflowV2CallRecord],
) -> Vec<&'a WorkflowV2CallRecord> {
    let mut found: Vec<&WorkflowV2CallRecord> = stored
        .iter()
        .filter(|record| {
            record.invalidated_by.is_none()
                && super::super::remediation_contract_string(&record.call, "stage")
                    == Some("verify")
                && record.call.method != WorkflowV2HostMethod::Checkpoint
                && !super::view::confirm::is_confirmation(&record.call)
                && residual_key(&record.call) == Some(round.key.as_str())
        })
        .collect();
    found.sort_by_key(|record| super::finished(record));
    found
}

/// The gaps a pass left open: each gap of its rounds its judge did not
/// resolve, and each gap its rounds' verifiers recorded.
fn left_open(plan: &ResidualPlan, stored: &[WorkflowV2CallRecord]) -> Vec<Residual> {
    let mut open = Vec::new();
    for round in &plan.rounds {
        let verifiers = verifiers(round, stored);
        let judge = verifiers.last();
        for gap in &round.residuals {
            let resolved = judge.is_some_and(|judge| match disposition_of(judge, &gap.id) {
                Some(said) => said == Disposition::Resolved,
                None => is_reusable_status(judge.status),
            });
            if !resolved {
                open.push(gap.clone());
            }
        }
        for verifier in verifiers {
            open.extend(residuals_of(verifier, None));
        }
    }
    open
}

/// The open gaps of a pass, any severity, by id (a reworded gap is the same
/// gap).
fn open_ids(plan: &ResidualPlan, stored: &[WorkflowV2CallRecord]) -> BTreeSet<String> {
    left_open(plan, stored)
        .iter()
        .map(|gap| bare_id(&gap.id))
        .collect()
}

/// Pass `n` when the passes stalled: no round, and every gap the previous
/// pass left open reported -- quoted whole -- so the final gate blocks on it.
fn stalled(
    n: u64,
    previous: &ResidualPlan,
    stored: &[WorkflowV2CallRecord],
    open: &BTreeSet<String>,
) -> ResidualPlan {
    let ids = open.iter().cloned().collect::<Vec<_>>().join(", ");
    let mut plan = ResidualPlan::default();
    let mut seen = BTreeSet::new();
    for gap in left_open(previous, stored) {
        if seen.insert(gap.key()) {
            let why = format!(
                "the residual passes stopped before pass {n}: pass {} left the same open gaps ({}) as the pass before it, so no further pass makes progress; it stands as recorded: {:?}",
                n - 1,
                if ids.is_empty() { "none" } else { ids.as_str() },
                gap.description
            );
            plan.reported.push((gap, why));
        }
    }
    plan
}

#[path = "residual_pass_stops.rs"]
mod stops;
pub use stops::DEFAULT_MAX_RESIDUAL_PASSES;
pub(super) use stops::capped;

#[cfg(test)]
#[path = "residual_later_pass_tests.rs"]
mod tests;
