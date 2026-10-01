//! A recorded remediation answer replays only for the question it answered
//! (Batch H).
//!
//! A remediation's question is its prompt -- the findings, verbatim -- AND
//! the observation those findings were read from. The prompt alone does not
//! say which observation that was: a frozen acceptance check that still
//! fails after a fix landed prints exactly the text it printed before the
//! fix. Live on wf-0ddadd81 that let TASK-TRADING-012's first acceptance fix
//! (`review-remediate-task-trading-012-1-83`, which landed) answer, by
//! content, the same failure observed again on a tree that already held
//! that fix: resume after resume, `-87` and then `-81` replayed `-83`'s
//! answer, the check kept failing, and the task never got another attempt.
//! The drift rule's own premise -- "the next round is a different label and
//! a different contract" -- does not hold for the acceptance loop, which
//! re-asks round 1 of the same unit on every resume.
//!
//! So a remediation contract names the calls its question was formed from:
//! the review reduces (`sourceReduceCallIds`) and any live observation
//! (`observedBy`, e.g. the acceptance round whose failing checks are its
//! findings; the host never replays an acceptance round). A recorded answer
//! whose execution is OLDER than the latest recorded run of any of those
//! sources never saw what the question was formed from: it answers another
//! question, and the call is dispatched live. An answer recorded after its
//! sources -- every resume of review remediation, whose reduces are
//! replayed from the store and never re-recorded -- replays as before.
//!
//! A later (or escalated) round is formed from the earlier round's refusal:
//! when this session dispatched an earlier round's fix again, no earlier
//! session's answer to the later round saw that fix or its refusal, and
//! the later round runs too.
//!
//! "When the answer executed" is read pessimistically: the earliest of the
//! times its call record and the files of its branch say. A replay re-saves
//! the record (and a refile re-saves the outcome) with a later time than the
//! execution it restates, and a later time would read as an answer that
//! followed the observation. Nothing here knows a phase or PRD by name.

use std::path::Path;
use std::time::SystemTime;

use chrono::{DateTime, Utc};
use serde_json::Value;

use super::{
    WorkflowV2CallRecord, WorkflowV2HostCall, WorkflowV2ResultStore, remediation_contract,
};

/// Contract key naming live observations a remediation's findings came from.
pub const OBSERVED_BY_KEY: &str = "observedBy";

fn parse(at: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(at)
        .ok()
        .map(|at| at.with_timezone(&Utc))
}

fn strings(contract: &Value, key: &str) -> Vec<String> {
    contract
        .get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .collect()
}

/// The calls `call`'s question was formed from: the reduces and the live
/// observations its remediation contract names. Empty for any other call.
pub fn question_sources(call: &WorkflowV2HostCall) -> Vec<String> {
    let Some(contract) = remediation_contract(call) else {
        return Vec::new();
    };
    let mut sources = strings(contract, "sourceReduceCallIds");
    for id in strings(contract, OBSERVED_BY_KEY) {
        if !sources.contains(&id) {
            sources.push(id);
        }
    }
    sources
}

/// When the latest recorded run of any source of `call`'s question was
/// recorded, as the records on disk say now. A named live observation with
/// no record is taken as observed now: an answer cannot be shown to follow
/// what was never recorded. `None` when nothing is named or recorded:
/// nothing to be older than.
pub fn question_observed_at(
    call: &WorkflowV2HostCall,
    records: &[WorkflowV2CallRecord],
) -> Option<DateTime<Utc>> {
    let observed = remediation_contract(call)
        .map(|contract| strings(contract, OBSERVED_BY_KEY))
        .unwrap_or_default();
    if observed
        .iter()
        .any(|id| !records.iter().any(|record| record.call.id == *id))
    {
        return Some(Utc::now());
    }
    let sources = question_sources(call);
    records
        .iter()
        .filter(|record| sources.contains(&record.call.id))
        .filter_map(|record| {
            parse(&record.started_at)
                .into_iter()
                .chain(parse(&record.finished_at))
                .max()
        })
        .max()
}

/// Whether this session dispatched the fix of an earlier round of `call`'s
/// unit to an agent. A later (or escalated) round is asked because the
/// earlier round's verdict refused; that fix and its verdict are new to this
/// session, so no earlier session's answer to this round saw them.
pub fn earlier_round_dispatched(store: &WorkflowV2ResultStore, call: &WorkflowV2HostCall) -> bool {
    super::resume_drift::remediation_unit(call).is_some_and(|(unit, round)| {
        (1..round).any(|earlier| store.fix_dispatched(&format!("{unit}#{earlier}")))
    })
}

/// When `record`'s answer executed, pessimistically: the execution it
/// restates or, for a record an earlier session wrote, its finish as that
/// session left it -- whichever of those and its own times is earliest.
pub fn record_answered_at(
    store: &WorkflowV2ResultStore,
    record: &WorkflowV2CallRecord,
) -> Option<DateTime<Utc>> {
    [
        store.executed_finish(record).as_deref().and_then(parse),
        parse(&record.started_at),
        parse(&record.finished_at),
    ]
    .into_iter()
    .flatten()
    .min()
}

fn modified(path: &Path) -> Option<DateTime<Utc>> {
    let at: SystemTime = std::fs::metadata(path).ok()?.modified().ok()?;
    Some(DateTime::<Utc>::from(at))
}

fn record_of(
    store: &WorkflowV2ResultStore,
    call_id: &str,
    records: &[WorkflowV2CallRecord],
) -> Option<DateTime<Utc>> {
    records
        .iter()
        .find(|record| record.call.id == call_id)
        .and_then(|record| record_answered_at(store, record))
}

/// When the branch answer filed under `(call_id, item_id)` executed,
/// pessimistically: the earlier of its call record's answer time and the
/// outcome file. `None` when neither exists.
pub fn branch_answered_at(
    store: &WorkflowV2ResultStore,
    call_id: &str,
    item_id: &str,
    records: &[WorkflowV2CallRecord],
) -> Option<DateTime<Utc>> {
    let outcome = modified(&store.branch_outcome_path(call_id, item_id));
    [record_of(store, call_id, records), outcome]
        .into_iter()
        .flatten()
        .min()
}

/// Whether a recorded answer (from an earlier session) that executed at
/// `answered_at` answers another question than `call` asks now: an earlier
/// round of its unit was dispatched again this session, or the answer is
/// older than the latest observation the question was formed from. An
/// answer of unknown age is older. A question with no recorded source
/// constrains nothing.
pub fn answer_predates_question(
    store: &WorkflowV2ResultStore,
    call: &WorkflowV2HostCall,
    answered_at: Option<DateTime<Utc>>,
    records: &[WorkflowV2CallRecord],
) -> bool {
    if earlier_round_dispatched(store, call) {
        return true;
    }
    let Some(observed) = question_observed_at(call, records) else {
        return false;
    };
    answered_at.is_none_or(|answered| answered < observed)
}

/// Whether `recorded` asked what `asked` asks: the same prompt (the
/// findings, verbatim), the same declared targets and the same remediation
/// contract. For the reuse paths that waive the input hash (a completed
/// task's record under its own id), so that a waiver never stretches to
/// another question.
pub fn asks_the_same(recorded: &WorkflowV2HostCall, asked: &WorkflowV2HostCall) -> bool {
    recorded.options.task == asked.options.task
        && recorded.options.target_files == asked.options.target_files
        && remediation_contract(recorded) == remediation_contract(asked)
}

/// [`answer_predates_question`] for a recorded call answering `call`.
pub fn record_predates_question(
    store: &WorkflowV2ResultStore,
    call: &WorkflowV2HostCall,
    record: &WorkflowV2CallRecord,
    records: &[WorkflowV2CallRecord],
) -> bool {
    answer_predates_question(store, call, record_answered_at(store, record), records)
}

#[cfg(test)]
#[path = "resume_freshness_tests.rs"]
mod tests;
