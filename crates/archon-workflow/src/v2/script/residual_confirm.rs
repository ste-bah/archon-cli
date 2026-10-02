//! One read-only CONFIRMATION of a residual round no verifier judged.
//!
//! A host-planned round whose fix landed nothing is recorded done; a prelude
//! before the no-op verifier (a live run's round ran under one) then
//! filed a no-patch checkpoint and no verifier agent ever judged the tree.
//! An AGENT-recorded gap it carries (of any severity: every standing gap
//! blocks since Batch O) can then never be corroborated, so
//! the final gate would block it without having asked anyone -- a dead end,
//! even when another landing fixed it.
//!
//! After the three passes the prelude asks the host, on a checkpoint
//! carrying [`RESIDUAL_CONFIRM_MARKER`], which rounds need one. The host
//! lists each round of its three plans that was attempted (done), whose
//! latest fix landed nothing, that no verifier agent judged after that fix
//! (a confirmation itself excepted), and that carries a gap the host
//! did not build itself. Each gets exactly ONE read-only verifier of its
//! tasks under the id `verification-wave-<key>-confirm` (fixed, so a resume
//! replays it and never asks twice), with the round's own contract plus
//! `residual.confirm`; the dispatch check answers no other. It writes
//! nothing and plans no pass. Its verdict is the round's verifier: the gate
//! resolves the round only on its acceptance with every targeted gap
//! reported resolved (`residual_gate`'s no-op rule), and a refusal stands.
//! No pass's population reads a confirmation, so no plan moves.

use serde_json::{Value, json};

use super::super::super::{
    WorkflowV2CallExecution, WorkflowV2CallRecord, WorkflowV2HostCall, WorkflowV2HostMethod,
    WorkflowV2ResultStore, remediation_contract,
};
use super::super::{PlannedRound, RESIDUAL_CONTRACT_KEY, RoundKind, Wording, worded};
use super::{disposition_instruction, done_checkpoint_id, findings, session_records};
use crate::task_universe::WorkflowV2TaskUniverse;
use std::collections::BTreeMap;
use std::path::Path;

/// The checkpoint option that asks for the confirmations.
pub const RESIDUAL_CONFIRM_MARKER: &str = "residualConfirm";
/// Key of the list in that checkpoint's view.
pub const RESIDUAL_CONFIRM_KEY: &str = "residual_confirm";

/// Whether `call` is a round's confirmation.
pub fn is_confirmation(call: &WorkflowV2HostCall) -> bool {
    remediation_contract(call)
        .and_then(|contract| contract.get(RESIDUAL_CONTRACT_KEY))
        .and_then(|claimed| claimed.get("confirm"))
        == Some(&Value::Bool(true))
}

/// The call id a round's confirmation is filed under.
pub fn confirmation_call_id(key: &str) -> String {
    format!("verification-wave-{key}-confirm")
}

fn stage(record: &WorkflowV2CallRecord) -> Option<&str> {
    super::super::super::remediation_contract_string(&record.call, "stage")
}

fn at(store: &WorkflowV2ResultStore, record: &WorkflowV2CallRecord) -> String {
    store
        .executed_finish(record)
        .unwrap_or_else(|| record.finished_at.clone())
}

/// Whether `round` needs its one confirmation, from its stored records.
fn needs_confirmation(
    store: &WorkflowV2ResultStore,
    round: &PlannedRound,
    stored: &[WorkflowV2CallRecord],
) -> bool {
    if round.kind == RoundKind::Adjudication
        || !round.residuals.iter().any(|gap| !gap.host_built)
        || store
            .load_call_record(&done_checkpoint_id(&round.key))
            .ok()
            .flatten()
            .is_none()
    {
        return false;
    }
    let own: Vec<&WorkflowV2CallRecord> = stored
        .iter()
        .filter(|record| {
            record.invalidated_by.is_none()
                && super::super::second_pass::residual_key(&record.call) == Some(round.key.as_str())
        })
        .collect();
    let Some(fix) = own
        .iter()
        .filter(|record| {
            stage(record) == Some("remediate")
                && record.call.method != WorkflowV2HostMethod::Checkpoint
        })
        .max_by_key(|record| at(store, record))
    else {
        return false;
    };
    let fixed_at = at(store, fix);
    super::super::super::remediation_escalation::landed_nothing(&fix.result.data)
        && !own.iter().any(|record| {
            stage(record) == Some("verify")
                && record.call.method != WorkflowV2HostMethod::Checkpoint
                && !is_confirmation(&record.call)
                && at(store, record) >= fixed_at
        })
}

/// Every round of the three plans (over this session's records) that needs
/// its confirmation.
fn rounds_needing(
    store: &WorkflowV2ResultStore,
    universe: Option<&WorkflowV2TaskUniverse>,
    root: Option<&Path>,
) -> crate::WorkflowResult<Vec<PlannedRound>> {
    let records = session_records(store);
    let refs: Vec<&WorkflowV2CallRecord> = records.iter().collect();
    let stored = store.load_call_records()?;
    let mut rounds: BTreeMap<String, PlannedRound> = BTreeMap::new();
    // Batch O2: every pass the run reached, the later ones included.
    let reached = stored
        .iter()
        .filter_map(|record| super::super::slot_pass(&record.call))
        .max()
        .unwrap_or(3)
        .max(3);
    for plan in super::super::pass_plans(reached, &refs, store, universe, root) {
        for round in plan.rounds {
            rounds.entry(round.key.clone()).or_insert(round);
        }
    }
    Ok(rounds
        .into_values()
        .filter(|round| needs_confirmation(store, round, &stored))
        .collect())
}

/// The verifier's prompt: every gap whole, and what to report of each (the
/// live path builds it through [`claim_for`], which keeps a recorded cut
/// wording).
#[cfg(test)]
pub fn confirmation_claim(round: &PlannedRound) -> String {
    claim_worded(round, Wording::Whole)
}

/// [`confirmation_claim`], or the cut text an earlier binary dispatched the
/// round's confirmation under (`residual_wording`), as `stored` records it.
fn claim_for(round: &PlannedRound, stored: &[WorkflowV2CallRecord]) -> String {
    worded(stored, &round.key, |wording| claim_worded(round, wording))
}

fn claim_worded(round: &PlannedRound, wording: Wording) -> String {
    let tasks = round.tasks.iter().cloned().collect::<Vec<_>>().join(", ");
    let mut summaries: BTreeMap<&str, String> = BTreeMap::new();
    for residual in &round.residuals {
        summaries
            .entry(residual.recorded_by.as_str())
            .or_insert_with(|| wording.summary(&residual.recorded_summary));
    }
    format!(
        "Read-only CONFIRMATION (host round {}): the round's fix landed no patch, so no verifier has judged the gaps it carries on the tree since. Judge the repository as it is NOW. The gaps (verbatim):\n{}\nThe recording verifiers' summaries (verbatim):\n{}\nAccept only if every one of these gaps no longer holds on the current tree AND each of {tasks}'s acceptance criteria and must-pass baseline tests pass; if a gap still holds, refuse.\n{}",
        round.key,
        Value::Array(findings(round, wording)),
        json!(summaries),
        disposition_instruction(round)
    )
}

/// The confirmation's remediation contract: the round's own, read-only.
fn contract(round: &PlannedRound) -> Value {
    let ids: Vec<&String> = round.tasks.iter().collect();
    let mut residual = json!({"key": round.key, "files": round.files, "confirm": true});
    if round.pass >= 2 {
        residual["pass"] = json!(round.pass);
    }
    let mut contract = json!({"version": 1, "stage": "verify", "round": 1, "maxRounds": 1,
        "sourceReduceCallIds": ["adversarial-review-reduce", "coverage-audit-reduce"],
        "contest": round.key, "residual": residual});
    if ids.len() > 1 {
        let joined: Vec<&str> = ids.iter().map(|id| id.as_str()).collect();
        contract["taskId"] = json!(format!("cross:{}", joined.join("+")));
        contract["taskIds"] = json!(ids);
    } else {
        contract["taskId"] = json!(ids.first());
    }
    contract
}

/// The list the confirmation checkpoint's view carries.
pub(in crate::v2::script::residual_plan) fn confirmation_view(
    store: &WorkflowV2ResultStore,
    universe: Option<&WorkflowV2TaskUniverse>,
    root: Option<&Path>,
) -> crate::WorkflowResult<Vec<Value>> {
    let stored = store.load_call_records()?;
    Ok(rounds_needing(store, universe, root)?
        .iter()
        .map(|round| {
            let attempted = store
                .load_call_record(&confirmation_call_id(&round.key))
                .ok()
                .flatten()
                .is_some();
            json!({"source": "host", "key": round.key, "task_ids": round.tasks,
                "claim": claim_for(round, &stored), "contract": contract(round),
                "attempted": attempted})
        })
        .collect())
}

/// Why the confirmation call `execution` may not be answered, or `None`
/// when it is exactly one the host lists now.
pub(in crate::v2::script::residual_plan) fn confirmation_refusal(
    execution: &WorkflowV2CallExecution,
    store: &WorkflowV2ResultStore,
    universe: Option<&WorkflowV2TaskUniverse>,
    root: Option<&Path>,
    key: &str,
) -> Option<String> {
    let call = &execution.call;
    let rounds = match rounds_needing(store, universe, root) {
        Ok(rounds) => rounds,
        Err(error) => return Some(format!("the host could not read its records: {error}")),
    };
    let Some(round) = rounds.into_iter().find(|round| round.key == key) else {
        return Some(format!("round `{key}` needs no confirmation"));
    };
    if call.id != confirmation_call_id(key)
        || call.write_mode.is_some()
        || call.method == WorkflowV2HostMethod::Checkpoint
    {
        return Some("a confirmation is one read-only verifier under the host's id".into());
    }
    if remediation_contract(call) != Some(&contract(&round)) {
        return Some("its contract is not the confirmation's".into());
    }
    let prompt = call.options.task.as_deref().unwrap_or_default();
    let stored = match store.load_call_records() {
        Ok(stored) => stored,
        Err(error) => return Some(format!("the host could not read its records: {error}")),
    };
    if !prompt.contains(&claim_for(&round, &stored)) {
        return Some("its prompt does not carry the host's claim".into());
    }
    let items = execution.input["source_data"].as_array();
    let tasks: std::collections::BTreeSet<String> = items
        .filter(|items| items.len() == 1)
        .and_then(|items| items[0]["canonical_task_ids"].as_array())
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect();
    (tasks != round.tasks).then(|| "it does not verify exactly the round's tasks".into())
}

#[cfg(test)]
#[path = "residual_confirm_tests.rs"]
mod tests;
