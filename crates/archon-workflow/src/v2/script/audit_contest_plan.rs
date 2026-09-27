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
//!
//! Issue-122: a remediation whose fix landed changes the path, so the pair it
//! answered can drop out of the report's contests before its verifier ran;
//! it is still listed, as it was asked, until its done checkpoint exists.
//! Every entry with a recorded remediation also carries that unit's place in
//! the prelude's call ordinal and the finding its fix was filed under, so a
//! resumed remediation replays the fix that landed and runs only its
//! verifier.

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
/// the audit's latest report now -- and, before them, one per contest
/// remediation a stopped session left unfinished whose pair the report no
/// longer names ([`unfinished_elsewhere`]).
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
    let records = store.load_call_records().unwrap_or_default();
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
            let mut entry = json!({
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
            });
            with_recorded_unit(
                &mut entry,
                &records,
                declarer,
                &contest.declared_path,
                &contest.state,
            );
            plan.push(entry);
        }
    }
    // Before the pairs asked now: what a stopped session left is finished
    // first, on the tree its own landing made.
    let mut unfinished = unfinished_elsewhere(store, &records, &plan);
    unfinished.extend(plan);
    unfinished
}

/// Issue-122: the prelude's key for the remediation unit of a pair: its
/// `keyHash`, FNV-1a over UTF-16 code units exactly as the script computes it
/// (the same bytes as [`fnv`] for ASCII).
fn unit_key(declarer: &str, path: &str, state: &str) -> String {
    let mut hash: u32 = 0x811c_9dc5;
    for unit in format!("{path}#{state}#{declarer}").encode_utf16() {
        hash ^= u32::from(unit);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    format!("{hash:08x}")
}

/// The recorded remediation unit of a pair, onto its plan entry: where its
/// calls sat in the ordinal, and the finding its first fix was filed under
/// (so a resumed remediation files its fix under the same input).
fn with_recorded_unit(
    entry: &mut Value,
    records: &[WorkflowV2CallRecord],
    declarer: &str,
    path: &str,
    state: &str,
) {
    let unit = unit_key(declarer, path, state);
    let ordinals = super::resume_ordinals::unit_ordinals(records, &unit);
    entry["fix_ordinal"] = json!(ordinals.fix_ordinal);
    entry["resume_ordinal"] = json!(ordinals.resume_ordinal);
    entry["finding_json"] = json!(recorded_finding(records, &unit, ordinals.fix_ordinal));
}

/// The findings JSON the unit's latest first fix was filed under: the one
/// line after the prelude's `Findings (verbatim):` marker, when it parses
/// as a list of one finding (a cut one does not).
fn recorded_finding(
    records: &[WorkflowV2CallRecord],
    unit: &str,
    fix_ordinal: Option<u64>,
) -> Option<String> {
    let fix = records.iter().find(|record| {
        record.call.write_mode.is_some()
            && super::remediation_contract_string(&record.call, "contest") == Some(unit)
            && super::remediation_contract_string(&record.call, "stage") == Some("remediate")
            && super::resume_ordinals::ordinal_of(&record.call.id) == fix_ordinal
            && fix_ordinal.is_some()
    })?;
    let prompt = fix.call.options.task.as_deref()?;
    let (_, after) = prompt.split_once(FINDINGS_MARKER)?;
    let line = after.split('\n').next()?;
    let parsed: Value = serde_json::from_str(line).ok()?;
    (parsed
        .as_array()
        .is_some_and(|list| list.len() == 1 && list[0].is_object()))
    .then(|| line.to_string())
}

/// The marker the prelude's remediation prompt puts before its findings.
const FINDINGS_MARKER: &str = "Findings (verbatim):\n";

/// Issue-122a: a contest remediation a stopped session left between its
/// landed fix and its verifier, when the pair it answered is no longer in the
/// report's contests -- its own landing changed the path, so the contest the
/// host names now is a different one, and nothing would ever verify what
/// landed. Each confirmation the host recorded names its pair
/// ([`AUDIT_CONTEST_OPTION`]); one whose unit has a recorded fix and is in
/// no entry of `current` is listed with the pair as it was asked --
/// `remediate` until its done checkpoint exists, then `attempted` (so a
/// later resume still takes its place in the ordinal).
fn unfinished_elsewhere(
    store: &WorkflowV2ResultStore,
    records: &[WorkflowV2CallRecord],
    current: &[Value],
) -> Vec<Value> {
    let mut found: Vec<Value> = Vec::new();
    for record in records {
        let Some(claimed) = record.call.options.extra.get(AUDIT_CONTEST_OPTION) else {
            continue;
        };
        let field = |key: &str| claimed.get(key).and_then(Value::as_str).unwrap_or_default();
        let (path, state, declarer) = (field("path"), field("state"), field("declarer"));
        let id = confirmation_id(declarer, path, state);
        if record.call.id != format!("verification-wave-{id}")
            || current
                .iter()
                .chain(&found)
                .any(|entry| entry["confirmation_id"] == id.as_str())
        {
            continue;
        }
        let unit = unit_key(declarer, path, state);
        // Only a remediation whose fix LANDED left anything to verify; one
        // whose fix failed or changed nothing is not re-run against a pair
        // the report no longer names.
        let ordinals = super::resume_ordinals::unit_ordinals(records, &unit);
        let landed = records.iter().any(|fix| {
            fix.call.write_mode.is_some()
                && super::remediation_contract_string(&fix.call, "contest") == Some(unit.as_str())
                && super::remediation_contract_string(&fix.call, "stage") == Some("remediate")
                && super::is_reusable_status(fix.status)
                && !super::remediation_escalation::landed_nothing(&fix.result.data)
        });
        if ordinals.fix_ordinal.is_none() || !landed {
            continue;
        }
        let done = store
            .load_call_record(&done_checkpoint_id(&id))
            .ok()
            .flatten()
            .is_some();
        let mut entry = json!({
            "source": "host",
            "path": path,
            "state": state,
            "declarer": declarer,
            "confirmation_id": id,
            "attempted": done,
            "remediate": !done,
            "unfinished": true,
            "refusal_summary": record.result.summary,
        });
        with_recorded_unit(&mut entry, records, declarer, path, state);
        found.push(entry);
    }
    found.sort_by(|a, b| {
        a["confirmation_id"]
            .as_str()
            .cmp(&b["confirmation_id"].as_str())
    });
    found
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
                // Only a pair still to be asked: a finished one, or one
                // whose refusal's remediation is still owed, is never asked
                // again.
                && entry["attempted"] != true
                && entry["remediate"] != true
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
