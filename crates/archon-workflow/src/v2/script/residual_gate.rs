//! Issue-117: residual gaps at the authored run's final gate.
//!
//! Every in-scope gap an accepted remediation verifier of the executed plan
//! recorded is accounted here, from host records alone:
//!
//! - one a host-planned round carried is RESOLVED when that round's last fix
//!   landed and was accepted, and its last verifier AGENT accepted after it,
//!   for every task of the round;
//! - everything else stands and is reported by name: the plan could not
//!   route it, its round did not resolve it, or it was recorded after the
//!   pre-acceptance slot (acceptance-stage and residual-round verifiers,
//!   where nothing follows to route it).
//!
//! A gap that stands blocks green when it is HIGH and is listed as a warning
//! when it is MEDIUM. This is the existing severity gate's rule for a finding
//! the host cannot attribute to a task (`v3_run_outcome_findings`: blocks
//! unless its severity is on the low-impact list), placed one step lower for
//! one reason: a residual gap is recorded by a verifier that ACCEPTED, so the
//! verifier itself judged a medium gap no reason to refuse; a high gap under
//! an acceptance is a contradiction the host must not wave through. The
//! severity reading is the same (trimmed, case-insensitive); a gap of no or
//! another severity is out of scope, as the host's own `review`/`info`
//! bookkeeping gaps are -- except the ones the host itself flagged as naming
//! only undeclared paths (Issue-81), whose severity it replaced: those are
//! listed by name.
//!
//! A REVIEW round that resolved discharges the review unit it answered: the
//! terminal rule reads it as that unit's outcome.
//!
//! Issue-121: a HIGH gap stands whoever recorded it. A verifier that refused
//! is no weaker a witness than one that accepted, and its HIGH gaps were
//! read only when they were the host's own red tests: live on wf-0ddadd81 a
//! second-pass round's refused verifier recorded a regression that left
//! three declared must-pass tests red, and nothing weighed it. Such a gap
//! stands unless a round that carried it resolved it (the third pass plans
//! them) or the host's own later test runs answer it
//! (`residual_superseded`), which is listed as a note.

use std::collections::BTreeSet;
use std::path::Path;

use serde_json::Value;

use super::super::{
    WorkflowV2CallRecord, WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2ResultStore,
    call_fact, is_reusable_status, remediation_contract, remediation_contract_string,
};
use super::owed::routed_gaps;
use super::superseded::HostRuns;
use super::{
    PlannedRound, RESIDUAL_CONTRACT_KEY, Residual, ResidualSeverity, RoundKind, accepted_verdict,
    finished, flagged_of, is_residual_slot, is_second_pass_round, is_second_pass_slot,
    is_third_pass_round, is_third_pass_slot, plan_from, residuals_of, second_pass_plan,
    third_pass_plan,
};
use crate::task_universe::WorkflowV2TaskUniverse;
use crate::v2::verification::baseline_run_base::is_unowned_red_gap_id;

#[path = "residual_gate_rounds.rs"]
mod rounds;
use rounds::judge_rounds;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResidualVerdict {
    pub blocking: Vec<String>,
    pub notes: Vec<String>,
    /// Review remediation keys a resolved review round completed.
    pub discharged: BTreeSet<String>,
}

impl ResidualVerdict {
    fn weigh(&mut self, residual: &Residual, why: &str) {
        let files = if residual.files.is_empty() {
            String::new()
        } else {
            format!(" on {}", residual.files.join(", "))
        };
        let text = format!("residual gap {}{files} stands: {why}", residual.label());
        match residual.severity {
            ResidualSeverity::High => self.blocking.push(text),
            ResidualSeverity::Medium => self.notes.push(format!("warning: {text}")),
        }
    }
}

/// The verdict over `calls`, the executed plan in script order.
pub fn residual_verdict(
    calls: &[WorkflowV2HostCall],
    store: &WorkflowV2ResultStore,
    universe: Option<&WorkflowV2TaskUniverse>,
    repository_root: Option<&Path>,
) -> ResidualVerdict {
    // A call id seen twice keeps its LAST position: the store holds only its
    // latest record.
    let mut last: Vec<(usize, &WorkflowV2HostCall)> = Vec::new();
    for (at, call) in calls.iter().enumerate() {
        last.retain(|(_, seen)| seen.id != call.id);
        last.push((at, call));
    }
    let slot = calls.iter().position(is_residual_slot);
    // Issue-118: the second pass's slot, when the script reached it.
    let second_slot = calls.iter().position(is_second_pass_slot);
    // Issue-121: and the third's.
    let third_slot = calls.iter().position(is_third_pass_slot);
    let mut before: Vec<WorkflowV2CallRecord> = Vec::new();
    let mut after: Vec<WorkflowV2CallRecord> = Vec::new();
    let mut before_second: Vec<WorkflowV2CallRecord> = Vec::new();
    let mut after_second: BTreeSet<String> = BTreeSet::new();
    let mut before_third: Vec<WorkflowV2CallRecord> = Vec::new();
    let mut after_third: BTreeSet<String> = BTreeSet::new();
    for (at, call) in last {
        let Some(record) = store
            .load_call_record(&call.id)
            .ok()
            .flatten()
            .filter(|record| record.invalidated_by.is_none())
        else {
            continue;
        };
        match second_slot {
            Some(second) if at < second => before_second.push(record.clone()),
            Some(_) => {
                after_second.insert(record.call.id.clone());
            }
            None => {}
        }
        match third_slot {
            Some(third) if at < third => before_third.push(record.clone()),
            Some(_) => {
                after_third.insert(record.call.id.clone());
            }
            None => {}
        }
        match slot {
            Some(slot) if at < slot => before.push(record),
            _ => after.push(record),
        }
    }
    let refs: Vec<&WorkflowV2CallRecord> = before.iter().collect();
    let plan = plan_from(&refs, universe, repository_root);
    // Issue-118: the second pass's plan, when the script reached its slot,
    // over the calls before it -- what its slot's view planned.
    let second = second_slot.map(|_| {
        let refs: Vec<&WorkflowV2CallRecord> = before_second.iter().collect();
        second_pass_plan(&refs, store, universe, repository_root)
    });
    let third = third_slot.map(|_| {
        let refs: Vec<&WorkflowV2CallRecord> = before_third.iter().collect();
        third_pass_plan(&refs, store, universe, repository_root)
    });
    // A round's own verifiers, from the store: a resume that skipped an
    // attempted round still weighs what its verifier recorded.
    let keys: BTreeSet<&str> = plan
        .rounds
        .iter()
        .chain(second.iter().flat_map(|second| &second.rounds))
        .chain(third.iter().flat_map(|third| &third.rounds))
        .map(|round| round.key.as_str())
        .collect();
    for record in store.load_call_records().unwrap_or_default() {
        let key = remediation_contract(&record.call)
            .and_then(|contract| contract.get(RESIDUAL_CONTRACT_KEY))
            .and_then(|claimed| claimed.get("key"))
            .and_then(Value::as_str)
            .map(str::to_string);
        if key.is_some_and(|key| keys.contains(key.as_str()))
            && !after.iter().any(|seen| seen.call.id == record.call.id)
        {
            after.push(record);
        }
    }
    // Every verifier judging after the slot -- the rounds' own and any
    // later one -- whose gaps a resolution must not recur in.
    let judges: Vec<&WorkflowV2CallRecord> = after
        .iter()
        .filter(|record| {
            record.invalidated_by.is_none()
                && remediation_contract_string(&record.call, "stage") == Some("verify")
                && record.call.method != WorkflowV2HostMethod::Checkpoint
        })
        .collect();
    let mut verdict = ResidualVerdict::default();
    let mut resolved: BTreeSet<String> = BTreeSet::new();
    let mut failed = judge_rounds(
        &plan.rounds,
        store,
        &judges,
        repository_root,
        &mut verdict,
        &mut resolved,
    );
    // What a pass reported stands unless a later pass's round that carried
    // it resolved it (the third pass plans what the first two reported).
    let mut standing: Vec<(Residual, String)> = plan.reported.clone();
    // The second pass: its rounds are judged the same way, by every verifier
    // after its slot; what it carried or reported is weighed there only.
    // Issue-121: the third pass exactly the same way.
    let mut later_known: BTreeSet<String> = BTreeSet::new();
    for later in second.iter().chain(&third) {
        later_known.extend(
            later
                .rounds
                .iter()
                .flat_map(|round| round.residuals.iter().map(Residual::key))
                .chain(later.reported.iter().map(|(residual, _)| residual.key())),
        );
        failed.extend(judge_rounds(
            &later.rounds,
            store,
            &judges,
            repository_root,
            &mut verdict,
            &mut resolved,
        ));
        standing.extend(later.reported.iter().cloned());
    }
    // A gap stands on its rounds' failures, or on a pass's report, only when
    // no round that carried it resolved it.
    let mut weighed = BTreeSet::new();
    for (residual, why) in standing.iter().chain(&failed) {
        if !resolved.contains(&residual.key()) && weighed.insert((residual.key(), why.clone())) {
            verdict.weigh(residual, why);
        }
    }
    let unrouted = if slot.is_some() {
        "it was recorded after the pre-acceptance slot, where no round can be planned for it"
    } else {
        "the run never reached the pre-acceptance slot, so no round was planned for it"
    };
    let too_late =
        "it was recorded after the second residual pass, where no round can be planned for it";
    let final_late = "harness cap exhausted: it was recorded after the third and final residual pass, where no round can be planned for it";
    let host = HostRuns::load(store);
    let mut seen = BTreeSet::new();
    for record in after.iter().filter(|record| {
        accepted_verdict(record)
            || (record.invalidated_by.is_none()
                && remediation_contract_string(&record.call, "stage") == Some("verify")
                && record.call.method != WorkflowV2HostMethod::Checkpoint)
    }) {
        // A refused verdict's gaps weigh where the host recorded them
        // itself -- the excused red tests it still owes (Issue-118) -- and,
        // since Issue-121, wherever they are HIGH.
        let accepted = accepted_verdict(record);
        for residual in residuals_of(record, repository_root) {
            let host_red = is_unowned_red_gap_id(&residual.id);
            if !accepted && !host_red && residual.severity != ResidualSeverity::High {
                continue;
            }
            if later_known.contains(&residual.key()) || !seen.insert(residual.key()) {
                continue;
            }
            if !accepted
                && !host_red
                && let Some(by) = host.superseded_by(&residual, record, None)
            {
                verdict.notes.push(format!(
                    "residual gap {} of refused verifier `{}` is answered: the later accepted verifier `{by}`'s host test runs passed every command its red tests failed in",
                    residual.label(),
                    record.call.id
                ));
                continue;
            }
            let why = if after_third.contains(&record.call.id) || is_third_pass_round(&record.call)
            {
                final_late
            } else if after_second.contains(&record.call.id) || is_second_pass_round(&record.call) {
                too_late
            } else {
                unrouted
            };
            verdict.weigh(&residual, why);
        }
    }
    // A red test an accepted verifier's baseline routed to its file's owner
    // weighs against that owner, unless a round resolved it or the host's
    // own latest run of it names it passed.
    let everything: Vec<&WorkflowV2CallRecord> = before.iter().chain(&after).collect();
    let stored = store.load_call_records().unwrap_or_default();
    for residual in routed_gaps(&everything, &stored, &keys, &host, None, repository_root) {
        if resolved.contains(&residual.key()) || later_known.contains(&residual.key()) {
            continue;
        }
        let owner = residual
            .unit_tasks
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        let why = if third_slot.is_some() {
            format!(
                "{owner} declares its file and answers for it; harness cap exhausted: no residual pass remains to plan a round of {owner}'s"
            )
        } else {
            format!(
                "{owner} declares its file and answers for it; no residual pass planned a round of {owner}'s"
            )
        };
        verdict.weigh(&residual, &why);
    }
    let flagged: Vec<String> = before
        .iter()
        .chain(&after)
        .filter(|record| accepted_verdict(record))
        .flat_map(flagged_of)
        .collect();
    if !flagged.is_empty() {
        verdict.notes.push(format!(
            "warning: {} residual gap(s) the host flagged as naming only paths no task declares (severity recorded as review): {}",
            flagged.len(),
            flagged.join(", ")
        ));
    }
    verdict
}

/// The live records of the round keyed `key`, from the store.
fn round_records(store: &WorkflowV2ResultStore, key: &str) -> Vec<WorkflowV2CallRecord> {
    store
        .load_call_records()
        .unwrap_or_default()
        .into_iter()
        .filter(|record| {
            record.invalidated_by.is_none()
                && remediation_contract(&record.call)
                    .and_then(|contract| contract.get(RESIDUAL_CONTRACT_KEY))
                    .and_then(|claimed| claimed.get("key"))
                    .and_then(Value::as_str)
                    == Some(key)
        })
        .collect()
}

/// The round's latest record of `stage`, by when it executed.
fn latest<'a>(
    store: &WorkflowV2ResultStore,
    own: &'a [WorkflowV2CallRecord],
    stage: &str,
) -> Option<&'a WorkflowV2CallRecord> {
    own.iter()
        .filter(|record| remediation_contract_string(&record.call, "stage") == Some(stage))
        .max_by_key(|record| executed(store, record))
}

/// Whether the round `round` resolved: its last fix landed and was accepted
/// and its last verifier agent accepted after it, for every task.
fn round_outcome(
    store: &WorkflowV2ResultStore,
    round: &PlannedRound,
    own: &[WorkflowV2CallRecord],
) -> Result<(), String> {
    let latest = |stage: &str| latest(store, own, stage);
    if round.kind == RoundKind::Adjudication {
        return adjudicated(round, latest("verify"));
    }
    let Some(fix) = latest("remediate") else {
        return Err("no round was recorded".to_string());
    };
    let Some(verify) = latest("verify") else {
        return Err(format!("its fix `{}` was never verified", fix.call.id));
    };
    if verify.call.method == WorkflowV2HostMethod::Checkpoint {
        return Err(format!(
            "its fix `{}` landed nothing, so no verifier judged it",
            fix.call.id
        ));
    }
    if executed(store, verify) < executed(store, fix) {
        return Err(format!(
            "its verifier `{}` ran before its fix `{}`",
            verify.call.id, fix.call.id
        ));
    }
    let fix_fact = call_fact(&fix.call, Some(fix));
    // Read from the record itself: a residual round's calls carry no
    // remediation role (`authored_call_role`), so the fact's own flag is
    // never set for them.
    let landed_nothing = super::super::remediation_escalation::landed_nothing(&fix.result.data);
    // A fix that landed nothing resolves nothing -- unless its round's own
    // verifier agent, judging after it, reported EVERY gap the round targets
    // resolved in its structured dispositions (a no-op round whose gaps the
    // tree no longer holds; the checks below still demand an accepted fix
    // and verifier for every task). A round targeting no gap never does.
    if landed_nothing && !noop_confirmed(round, verify) {
        return Err(format!(
            "its fix `{}` landed nothing, and its verifier `{}` did not report every gap it targets resolved",
            fix.call.id, verify.call.id
        ));
    }
    let verify_fact = call_fact(&verify.call, Some(verify));
    for (fact, which) in [(&fix_fact, "fix"), (&verify_fact, "verifier")] {
        for task in &round.tasks {
            match fact.task(task).or_else(|| fact.outcome()) {
                Some(outcome) if is_reusable_status(outcome.status) => {}
                Some(outcome) => {
                    return Err(format!(
                        "its {which} `{}` is {:?} for {task}",
                        fact.id, outcome.status
                    ));
                }
                None => return Err(format!("its {which} `{}` has no record", fact.id)),
            }
        }
    }
    Ok(())
}

/// Whether `verify` reported every gap of `round` resolved, each under an id
/// no other gap of the round shares.
fn noop_confirmed(round: &PlannedRound, verify: &WorkflowV2CallRecord) -> bool {
    use super::dispositions::{Disposition, bare_id, disposition_of};
    !round.residuals.is_empty()
        && round.residuals.iter().all(|residual| {
            round
                .residuals
                .iter()
                .filter(|other| bare_id(&other.id) == bare_id(&residual.id))
                .count()
                == 1
                && disposition_of(verify, &residual.id) == Some(Disposition::Resolved)
        })
}

/// An adjudication resolves its gaps only when its verifier agent accepted
/// for every task of the round AND recorded no HIGH gap of its own: a gap
/// the adjudicator records again, or any other, stands.
fn adjudicated(round: &PlannedRound, verify: Option<&WorkflowV2CallRecord>) -> Result<(), String> {
    let Some(verify) =
        verify.filter(|record| record.call.method != WorkflowV2HostMethod::Checkpoint)
    else {
        return Err("no adjudication was recorded".to_string());
    };
    let fact = call_fact(&verify.call, Some(verify));
    for task in &round.tasks {
        match fact.task(task).or_else(|| fact.outcome()) {
            Some(outcome) if is_reusable_status(outcome.status) => {}
            Some(outcome) => {
                return Err(format!(
                    "its adjudicator `{}` is {:?} for {task}",
                    fact.id, outcome.status
                ));
            }
            None => return Err(format!("its adjudicator `{}` has no record", fact.id)),
        }
    }
    let again: Vec<String> = residuals_of(verify, None)
        .into_iter()
        .filter(|residual| residual.severity == ResidualSeverity::High)
        .map(|residual| residual.label())
        .collect();
    if !again.is_empty() {
        return Err(format!(
            "its adjudicator `{}` recorded high gap(s) again: {}",
            fact.id,
            again.join(", ")
        ));
    }
    Ok(())
}

fn executed(store: &WorkflowV2ResultStore, record: &WorkflowV2CallRecord) -> i64 {
    store
        .executed_finish(record)
        .and_then(|at| chrono::DateTime::parse_from_rfc3339(&at).ok())
        .and_then(|at| at.timestamp_nanos_opt())
        .unwrap_or_else(|| finished(record))
}
