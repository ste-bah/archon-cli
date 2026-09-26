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

use std::collections::BTreeSet;
use std::path::Path;

use serde_json::Value;

use super::super::{
    WorkflowV2CallRecord, WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2ResultStore,
    call_fact, is_reusable_status, remediation_contract, remediation_contract_string,
};
use super::{
    PlannedRound, RESIDUAL_CONTRACT_KEY, Residual, ResidualSeverity, RoundKind, accepted_verdict,
    finished, flagged_of, is_residual_slot, plan_from, residuals_of,
};
use crate::task_universe::WorkflowV2TaskUniverse;

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
    let mut before: Vec<WorkflowV2CallRecord> = Vec::new();
    let mut after: Vec<WorkflowV2CallRecord> = Vec::new();
    for (at, call) in last {
        let Some(record) = store
            .load_call_record(&call.id)
            .ok()
            .flatten()
            .filter(|record| record.invalidated_by.is_none())
        else {
            continue;
        };
        match slot {
            Some(slot) if at < slot => before.push(record),
            _ => after.push(record),
        }
    }
    let refs: Vec<&WorkflowV2CallRecord> = before.iter().collect();
    let plan = plan_from(&refs, universe, repository_root);
    let mut verdict = ResidualVerdict::default();
    for round in &plan.rounds {
        match round_outcome(store, round) {
            Ok(()) => {
                verdict.notes.push(format!(
                    "host-planned {} round `{}` over {} resolved {}",
                    round.kind.as_str(),
                    round.key,
                    round.tasks.iter().cloned().collect::<Vec<_>>().join(", "),
                    described(round)
                ));
                if let Some(unit) = round
                    .unit_key
                    .as_ref()
                    .filter(|_| round.kind == RoundKind::Review)
                {
                    verdict.discharged.insert(unit.clone());
                }
            }
            Err(why) => {
                let why = format!(
                    "its host-planned {} round `{}` did not resolve it: {why}",
                    round.kind.as_str(),
                    round.key
                );
                for residual in &round.residuals {
                    verdict.weigh(residual, &why);
                }
                if round.kind == RoundKind::Review {
                    verdict.notes.push(format!(
                        "review unit {} was not completed by the host's ownership-expansion round: {why}",
                        round.unit_key.as_deref().unwrap_or_default()
                    ));
                }
            }
        }
    }
    for (residual, why) in &plan.reported {
        verdict.weigh(residual, why);
    }
    let unrouted = if slot.is_some() {
        "it was recorded after the pre-acceptance slot, where no round can be planned for it"
    } else {
        "the run never reached the pre-acceptance slot, so no round was planned for it"
    };
    // A round's own verifiers, from the store: a resume that skipped an
    // attempted round still weighs what its verifier recorded.
    let keys: BTreeSet<&str> = plan.rounds.iter().map(|round| round.key.as_str()).collect();
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
    let mut seen = BTreeSet::new();
    for record in after.iter().filter(|record| accepted_verdict(record)) {
        for residual in residuals_of(record, repository_root) {
            if seen.insert(residual.key()) {
                verdict.weigh(&residual, unrouted);
            }
        }
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

fn described(round: &PlannedRound) -> String {
    let gaps: Vec<String> = round.residuals.iter().map(Residual::label).collect();
    let mut text = if gaps.is_empty() {
        format!(
            "the refusal of review unit {}",
            round.unit_key.as_deref().unwrap_or_default()
        )
    } else {
        gaps.join(", ")
    };
    if !round.files.is_empty() {
        let files: Vec<&str> = round.files.iter().map(String::as_str).collect();
        text.push_str(&format!(" (granted {})", files.join(", ")));
    }
    text
}

/// Whether the round `round` resolved: its last fix landed and was accepted
/// and its last verifier agent accepted after it, for every task.
fn round_outcome(store: &WorkflowV2ResultStore, round: &PlannedRound) -> Result<(), String> {
    let records = store.load_call_records().unwrap_or_default();
    let mine: Vec<&WorkflowV2CallRecord> = records
        .iter()
        .filter(|record| {
            record.invalidated_by.is_none()
                && remediation_contract(&record.call)
                    .and_then(|contract| contract.get(RESIDUAL_CONTRACT_KEY))
                    .and_then(|claimed| claimed.get("key"))
                    .and_then(Value::as_str)
                    == Some(round.key.as_str())
        })
        .collect();
    let executed = |record: &WorkflowV2CallRecord| {
        store
            .executed_finish(record)
            .and_then(|at| chrono::DateTime::parse_from_rfc3339(&at).ok())
            .and_then(|at| at.timestamp_nanos_opt())
            .unwrap_or_else(|| finished(record))
    };
    let latest = |stage: &str| {
        mine.iter()
            .copied()
            .filter(|record| remediation_contract_string(&record.call, "stage") == Some(stage))
            .max_by_key(|record| executed(record))
    };
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
    if executed(verify) < executed(fix) {
        return Err(format!(
            "its verifier `{}` ran before its fix `{}`",
            verify.call.id, fix.call.id
        ));
    }
    let fix_fact = call_fact(&fix.call, Some(fix));
    if fix_fact.landed_nothing {
        return Err(format!("its fix `{}` landed nothing", fix.call.id));
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
