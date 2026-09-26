//! Issue-112b: resolve a contested declared path inside the run.
//!
//! A path is contested (`repository_audit::contest`) when one declaring
//! task's verified landing deleted it (or another re-delivered it after that)
//! and some declarer's own verification never accepted the tree as it now is.
//! Nothing in the run asked those declarers: the acceptance stage runs the
//! task set's frozen checks per CHECK (`acceptance_stage::AcceptanceCheckRecordV1`,
//! attributed through `owning_tasks`), recorded outside the call store, so it
//! is no verification of a task's contract and a task may own no check.
//!
//! So before the acceptance stage the prelude asks the host, through a
//! checkpoint carrying [`AUDIT_CONTESTS_MARKER`], which declarers are
//! unconfirmed. The host answers on that checkpoint's view -- computed from
//! the audit's latest report and the run's records at the moment of asking,
//! never persisted -- with one entry per (contested path, unconfirmed
//! declarer): the path, what the tree holds, who changed it, and the id of
//! the ONE read-only verification of that declarer the host will answer for
//! it ([`confirmation_id`]). The prelude dispatches exactly those; the host
//! refuses any confirmation call that is not one of them
//! ([`confirmation_refusal`]). A confirmation is an ordinary host-attributed
//! task verification, so the contest rule reads it like any other: accepted
//! on a commit whose state of the path matches, the declarer is confirmed.
//!
//! A pair is DONE once its confirmation was accepted, or was refused and the
//! remediation it was routed to reached its end (the prelude records
//! [`done_checkpoint_id`] after it returns): a done pair is `attempted` and
//! never asked again, so a still-contested path fails the final gate by
//! name. A refused confirmation whose remediation has no such record -- a
//! session that stopped in between -- is planned as `remediate`, with the
//! refusal's own summary, so a resume runs the remediation and does not ask
//! the verifier again.

use std::path::Path;

use serde_json::{Value, json};

use super::{WorkflowV2CallExecution, WorkflowV2CallRecord, WorkflowV2HostMethod};
use super::{WorkflowV2Result, WorkflowV2ResultStore};

/// The checkpoint option that asks the host for the contest plan.
pub const AUDIT_CONTESTS_MARKER: &str = "auditContests";
/// Key of the plan in that checkpoint's view.
pub const AUDIT_CONTESTS_KEY: &str = "audit_contests";
/// The option naming the contest a confirmation call answers.
pub const AUDIT_CONTEST_OPTION: &str = "auditContest";
use crate::repository_audit::contest::UNREADABLE_UNIVERSE as UNREADABLE;

fn slug(text: &str) -> String {
    let lowered: String = text
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let mut out = String::new();
    for part in lowered.split('-').filter(|part| !part.is_empty()) {
        if !out.is_empty() {
            out.push('-');
        }
        out.push_str(part);
    }
    out.chars().take(40).collect()
}

fn fnv(text: &str) -> String {
    let mut hash: u32 = 0x811c_9dc5;
    for byte in text.bytes() {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    format!("{hash:08x}")
}

/// The id the prelude files the confirmation of `declarer` for `path` in
/// `state` under (before the verifier wave prefix).
pub fn confirmation_id(declarer: &str, path: &str, state: &str) -> String {
    format!(
        "audit-confirm-{}-{}",
        slug(declarer),
        fnv(&format!("{path}#{state}"))
    )
}

/// The checkpoint the prelude records once a pair's routed remediation
/// returned.
pub fn done_checkpoint_id(confirmation_id: &str) -> String {
    format!("{confirmation_id}-done")
}

/// One entry per (contested path, unconfirmed declarer), as the host judges
/// the audit's latest report now.
pub fn contest_plan(store: &WorkflowV2ResultStore, repository_root: Option<&Path>) -> Vec<Value> {
    let (Some(root), Some(run_dir)) = (repository_root, store.root().parent()) else {
        return Vec::new();
    };
    let Ok(Some(state)) = crate::repository_audit::reuse::load_state(store) else {
        return Vec::new();
    };
    let Some(report) = state.ledger.history.last() else {
        return Vec::new();
    };
    let (_, contests) = crate::repository_audit::contest::judge(run_dir, report, root);
    let mut plan = Vec::new();
    for contest in contests {
        for declarer in contest
            .unconfirmed
            .iter()
            .filter(|d| d.as_str() != UNREADABLE)
        {
            let id = confirmation_id(declarer, &contest.declared_path, &contest.state);
            let load = |call_id: &str| store.load_call_record(call_id).ok().flatten();
            let confirmation = load(&format!("verification-wave-{id}"));
            let done = load(&done_checkpoint_id(&id)).is_some();
            let refused = confirmation
                .as_ref()
                .filter(|record| !super::is_reusable_status(record.status));
            let attempted = done || (confirmation.is_some() && refused.is_none());
            plan.push(json!({
                "source": "host",
                "path": contest.declared_path,
                "state": contest.state,
                "declarer": declarer,
                "deleted_by": contest.deleted_by,
                "deleted_in": contest.deletion_stage,
                "relanded_by": contest.relanded_by,
                "relanded_in": contest.relanded_stage,
                "confirmation_id": id,
                "attempted": attempted,
                "remediate": !attempted && refused.is_some(),
                "refusal_summary": refused.map(|record| record.result.summary.clone()),
            }));
        }
    }
    plan
}

fn asks_for_plan(record: &WorkflowV2CallRecord) -> bool {
    record.call.method == WorkflowV2HostMethod::Checkpoint
        && record.call.options.extra.get(AUDIT_CONTESTS_MARKER) == Some(&Value::Bool(true))
}

/// `result` with the host's contest plan, for the view of a checkpoint that
/// asked for it; `None` for every other record. The key is the host's alone.
pub fn with_contest_plan(
    record: &WorkflowV2CallRecord,
    result: &WorkflowV2Result,
    store: &WorkflowV2ResultStore,
    repository_root: Option<&Path>,
) -> Option<WorkflowV2Result> {
    let carried = result.data.get(AUDIT_CONTESTS_KEY).is_some();
    if !asks_for_plan(record) && !carried {
        return None;
    }
    let mut viewed = result.clone();
    if !viewed.data.is_object() {
        viewed.data = json!({});
    }
    if let Some(data) = viewed.data.as_object_mut() {
        data.remove(AUDIT_CONTESTS_KEY);
    }
    if asks_for_plan(record) {
        viewed.data[AUDIT_CONTESTS_KEY] = Value::Array(contest_plan(store, repository_root));
    }
    Some(viewed)
}

/// Why the confirmation call `execution` may not be answered, or `None` when
/// it is no such call or is exactly an entry of the host's plan now.
pub fn confirmation_refusal(
    execution: &WorkflowV2CallExecution,
    store: &WorkflowV2ResultStore,
    repository_root: Option<&Path>,
) -> Option<String> {
    let claimed = execution.call.options.extra.get(AUDIT_CONTEST_OPTION)?;
    let refused = |why: &str| {
        Some(format!(
            "contest confirmation `{}` refused: {why}",
            execution.call.id
        ))
    };
    if execution.call.write_mode.is_some()
        || execution.call.method == WorkflowV2HostMethod::Checkpoint
    {
        return refused("it is not a read-only verifier");
    }
    let field = |key: &str| claimed.get(key).and_then(Value::as_str).unwrap_or_default();
    let planned = contest_plan(store, repository_root)
        .into_iter()
        .find(|entry| {
            entry["path"] == field("path")
                && entry["state"] == field("state")
                && entry["declarer"] == field("declarer")
        });
    let Some(entry) = planned else {
        return refused("no contest of the host's names this path, state and declarer");
    };
    let id = entry["confirmation_id"].as_str().unwrap_or_default();
    if execution.call.id != format!("verification-wave-{id}") {
        return refused("its id is not the one the host planned");
    }
    let tasks: Vec<&str> = execution.input["source_data"][0]["canonical_task_ids"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    if tasks != [field("declarer")] {
        return refused("it does not verify exactly the unconfirmed declarer");
    }
    None
}

/// What the script is handed for a refused confirmation: no verdict.
pub fn refused_confirmation_result(reason: &str) -> WorkflowV2Result {
    WorkflowV2Result {
        status: super::WorkflowV2Status::Failed,
        summary: reason.to_string(),
        data: json!({ "confirmation_refused": reason }),
        ..WorkflowV2Result::default()
    }
}

#[cfg(test)]
#[path = "audit_contest_plan_tests.rs"]
mod tests;
