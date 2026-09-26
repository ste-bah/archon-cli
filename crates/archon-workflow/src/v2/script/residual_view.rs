//! Issue-117: the residual plan on the pre-acceptance checkpoint's view.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{Value, json};

use super::super::{
    WorkflowV2CallRecord, WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2Result,
    WorkflowV2ResultStore,
};
use super::{
    DESCRIPTION_CHARS, PlannedRound, RESIDUAL_GAPS_KEY, RESIDUAL_GAPS_MARKER, RoundKind,
    SUMMARY_CHARS, clip, plan_from,
};
use crate::task_universe::WorkflowV2TaskUniverse;

/// The checkpoint the prelude records once a round's remediation returned.
pub fn done_checkpoint_id(key: &str) -> String {
    format!("{key}-done")
}

/// This session's records, as the plan at the slot reads them.
pub fn session_records(store: &WorkflowV2ResultStore) -> Vec<WorkflowV2CallRecord> {
    store
        .load_call_records()
        .unwrap_or_default()
        .into_iter()
        .filter(|record| store.in_session(&record.call.id))
        .collect()
}

/// The plan as the pre-acceptance checkpoint's view carries it.
pub fn residual_plan_view(
    store: &WorkflowV2ResultStore,
    universe: Option<&WorkflowV2TaskUniverse>,
    root: Option<&Path>,
) -> Vec<Value> {
    let records = session_records(store);
    let refs: Vec<&WorkflowV2CallRecord> = records.iter().collect();
    plan_from(&refs, universe, root)
        .rounds
        .iter()
        .map(|round| round_view(round, store))
        .collect()
}

pub fn round_view(round: &PlannedRound, store: &WorkflowV2ResultStore) -> Value {
    let attempted = store
        .load_call_record(&done_checkpoint_id(&round.key))
        .ok()
        .flatten()
        .is_some();
    let claim = round_claim(round);
    // Checked before anything is dispatched: the prompt the prelude builds
    // from this claim (quoted inside a JSON finding, then quoted again)
    // carries everything the dispatch check reads. A round it would not
    // carry is not offered: it is reported, never recorded done.
    let quoted = json!([{ "claim": json!([{ "claim": claim }]).to_string() }]).to_string();
    let dispatchable = super::dispatch::unquoted_in(&quoted, round).is_none();
    json!({
        "source": "host",
        "key": round.key,
        "kind": round.kind.as_str(),
        "task_ids": round.tasks,
        "expansion_files": round.files,
        "severity": round.severity().as_str(),
        "findings": findings(round),
        "claim": claim,
        "dispatchable": dispatchable,
        "unit_key": round.unit_key,
        "refusal": round.refusal,
        "attempted": attempted,
    })
}

fn findings(round: &PlannedRound) -> Vec<Value> {
    round
        .residuals
        .iter()
        .map(|residual| {
            json!({
                "id": residual.id,
                "severity": residual.severity.as_str(),
                "recorded_by": residual.recorded_by,
                "paths": residual.files,
                "description": clip(&residual.description, DESCRIPTION_CHARS),
            })
        })
        .collect()
}

/// The text a round's finding carries (and an adjudication's prompt), built
/// by the host: every gap whole -- id, severity, recording call, paths and
/// its description up to [`DESCRIPTION_CHARS`] -- and each recording
/// verifier's summary once. A round holds at most a handful of gaps
/// (`MAX_GAPS_PER_ROUND`), so nothing here is ever cut to fit.
pub fn round_claim(round: &PlannedRound) -> String {
    let tasks = round.tasks.iter().cloned().collect::<Vec<_>>().join(", ");
    let files = round.files.iter().cloned().collect::<Vec<_>>().join(", ");
    let mut summaries: BTreeMap<&str, String> = BTreeMap::new();
    for residual in &round.residuals {
        summaries
            .entry(residual.recorded_by.as_str())
            .or_insert_with(|| clip(&residual.recorded_summary, SUMMARY_CHARS));
    }
    let gaps = Value::Array(findings(round)).to_string();
    let summaries = json!(summaries).to_string();
    let scope = if files.is_empty() {
        String::new()
    } else {
        format!(
            " This one bounded round may ALSO write {files}, which no task declares, and nothing else outside {tasks}'s own files."
        )
    };
    match round.kind {
        RoundKind::Review => format!(
            "Host round {}: the review remediation of {} was refused because the change it needs lies in files no task declares.{scope} Make that remediation's findings hold. The refused verifier's judgment and the unit's findings (their words, quoted):\n{}",
            round.key,
            round.unit_key.as_deref().unwrap_or_default(),
            round.refusal.clone().unwrap_or_default()
        ),
        RoundKind::Adjudication => format!(
            "Read-only ADJUDICATION (host round {}) of residual gap(s) an accepted verifier recorded against {tasks} that name no file a round could write. Judge the repository as it is NOW. The gaps (verbatim):\n{gaps}\nThe recording verifiers' summaries (verbatim):\n{summaries}\nAccept only if every one of these gaps is resolved or invalid on the current tree AND each of {tasks}'s acceptance criteria and must-pass baseline tests pass; if a gap still holds, refuse, or record it again as a residual gap.",
            round.key
        ),
        RoundKind::Owned | RoundKind::Expansion => format!(
            "Host round {}: accepted verifiers recorded these residual gaps; the host routed them to {tasks}.{scope} The gaps (verbatim):\n{gaps}\nThe recording verifiers' summaries (verbatim):\n{summaries}\nFix exactly what they name, keeping every one of {tasks}'s acceptance criteria and must-pass baseline tests passing.",
            round.key
        ),
    }
}

fn asks_for_plan(record: &WorkflowV2CallRecord) -> bool {
    record.call.method == WorkflowV2HostMethod::Checkpoint
        && record.call.options.extra.get(RESIDUAL_GAPS_MARKER) == Some(&Value::Bool(true))
}

/// Whether `call` is the pre-acceptance checkpoint asking for the plan.
pub fn is_residual_slot(call: &WorkflowV2HostCall) -> bool {
    call.method == WorkflowV2HostMethod::Checkpoint
        && call.options.extra.get(RESIDUAL_GAPS_MARKER) == Some(&Value::Bool(true))
}

/// `result` with the host's residual plan, for the view of the checkpoint
/// that asked for it; `None` for every other record. The key is the host's
/// alone: one already in the data is dropped.
pub fn with_residual_plan(
    record: &WorkflowV2CallRecord,
    result: &WorkflowV2Result,
    store: &WorkflowV2ResultStore,
    universe: Option<&WorkflowV2TaskUniverse>,
    root: Option<&Path>,
) -> Option<WorkflowV2Result> {
    let carried = result.data.get(RESIDUAL_GAPS_KEY).is_some();
    if !asks_for_plan(record) && !carried {
        return None;
    }
    let mut viewed = result.clone();
    if !viewed.data.is_object() {
        viewed.data = json!({});
    }
    if let Some(data) = viewed.data.as_object_mut() {
        data.remove(RESIDUAL_GAPS_KEY);
    }
    if asks_for_plan(record) {
        viewed.data[RESIDUAL_GAPS_KEY] = Value::Array(residual_plan_view(store, universe, root));
    }
    Some(viewed)
}
