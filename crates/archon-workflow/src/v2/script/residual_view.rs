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

/// The second pass's plan as its slot's view carries it (Issue-118).
pub fn second_pass_view(
    store: &WorkflowV2ResultStore,
    universe: Option<&WorkflowV2TaskUniverse>,
    root: Option<&Path>,
) -> Vec<Value> {
    let records = session_records(store);
    let refs: Vec<&WorkflowV2CallRecord> = records.iter().collect();
    super::second_pass_plan(&refs, store, universe, root)
        .rounds
        .iter()
        .map(|round| round_view(round, store))
        .collect()
}

/// The third pass's plan as its slot's view carries it (Issue-121). Each
/// round's claim keeps the wording its already-dispatched calls carry (see
/// [`dispatched_with_earlier_wording`]); a store that cannot be read is an
/// error, never "nothing dispatched".
pub fn third_pass_view(
    store: &WorkflowV2ResultStore,
    universe: Option<&WorkflowV2TaskUniverse>,
    root: Option<&Path>,
) -> crate::WorkflowResult<Vec<Value>> {
    let stored = store.load_call_records()?;
    let records = session_records(store);
    let refs: Vec<&WorkflowV2CallRecord> = records.iter().collect();
    Ok(super::third_pass_plan(&refs, store, universe, root)
        .rounds
        .iter()
        .map(|round| {
            let earlier = round.pass < 3 || dispatched_with_earlier_wording(round, &stored);
            round_view_worded(round, store, earlier)
        })
        .collect())
}

/// Whether a call already recorded for `round` (not a checkpoint) was
/// dispatched with the earlier wording: its recorded call carries the
/// earlier claim's host-written opening for THIS round's key. Keyed on the
/// round's own key, which no earlier verifier was ever shown, so a gap
/// description or summary quoting an earlier round's claim cannot match. A
/// round dispatched with the current wording, or not dispatched at all,
/// keeps the current wording, so a resume rebuilds exactly the recorded
/// input either way.
fn dispatched_with_earlier_wording(round: &PlannedRound, stored: &[WorkflowV2CallRecord]) -> bool {
    let key = round.key.as_str();
    let openings = [
        format!("Host round {key}: accepted verifiers recorded these residual gaps"),
        format!("(host round {key}) of residual gap(s) an accepted verifier recorded against"),
    ];
    stored.iter().any(|record| {
        record.call.method != WorkflowV2HostMethod::Checkpoint
            && super::second_pass::residual_key(&record.call) == Some(key)
            && serde_json::to_string(&record.call)
                .is_ok_and(|call| openings.iter().any(|opening| call.contains(opening)))
    })
}

pub fn round_view(round: &PlannedRound, store: &WorkflowV2ResultStore) -> Value {
    round_view_worded(round, store, round.pass < 3)
}

fn round_view_worded(round: &PlannedRound, store: &WorkflowV2ResultStore, earlier: bool) -> Value {
    let attempted = store
        .load_call_record(&done_checkpoint_id(&round.key))
        .ok()
        .flatten()
        .is_some();
    let claim = round_claim_worded(round, earlier);
    // Issue-122: where the round's calls sat in the prelude's ordinal.
    let ordinals = super::super::resume_ordinals::unit_ordinals(
        &store.load_call_records().unwrap_or_default(),
        &round.key,
    );
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
        "disposition_instruction": disposition_instruction(round),
        "fix_ordinal": ordinals.fix_ordinal,
        "resume_ordinal": ordinals.resume_ordinal,
    })
}

/// What the round's verifier is told to report of each gap the round
/// targets (the gate reads it from that verifier's record alone); empty for
/// a round that targets no gap.
pub fn disposition_instruction(round: &PlannedRound) -> String {
    let mut ids: Vec<&str> = Vec::new();
    for residual in &round.residuals {
        let bare = super::dispositions::bare_id(&residual.id);
        if !ids
            .iter()
            .any(|id| super::dispositions::bare_id(id) == bare)
        {
            ids.push(residual.id.as_str());
        }
    }
    if ids.is_empty() {
        return String::new();
    }
    format!(
        "GAP DISPOSITIONS (required): this host round targets the residual gap id(s) {ids}. Your result MUST carry, inside its \"data\" object, \"{key}\": [{{\"gap_id\": \"<id>\", \"status\": \"resolved\"}}] with exactly one entry per id above, written exactly as above: status \"resolved\" only when you established on the tree you judged that the gap no longer holds, \"open\" when it still holds or you could not establish that. Any new, separate problem you find, even in the same file, is not a disposition: record it in residual_gaps as usual, under its own id and at its own severity. A targeted gap that still holds is \"open\" (you may also record it again under its own id), never recorded under a new id.",
        ids = json!(ids),
        key = super::GAP_DISPOSITIONS_KEY,
    )
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
    round_claim_worded(round, round.pass < 3)
}

/// [`round_claim`], with the first two passes' "accepted verifiers" wording
/// when `earlier_wording`.
fn round_claim_worded(round: &PlannedRound, earlier_wording: bool) -> String {
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
    // Issue-121: the third pass plans HIGH gaps a second-pass round's
    // verifier recorded WHATEVER its verdict, so "accepted verifiers" would
    // be false there. The first two passes' wording is unchanged, and so is a
    // round's already dispatched under it: claims are dispatched call inputs
    // a resumed run replays.
    let (adjudicated, recorded) = if !earlier_wording {
        (
            "HIGH residual gap(s) a verifier of the host's second-pass rounds recorded (whatever its verdict: a refused verifier's HIGH gap counts)",
            "verifiers of the host's second-pass rounds recorded these HIGH residual gaps, whatever their verdict (a refused verifier's HIGH gap counts)",
        )
    } else {
        (
            "residual gap(s) an accepted verifier recorded",
            "accepted verifiers recorded these residual gaps",
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
            "Read-only ADJUDICATION (host round {}) of {adjudicated} against {tasks} that name no file a round could write. Judge the repository as it is NOW. The gaps (verbatim):\n{gaps}\nThe recording verifiers' summaries (verbatim):\n{summaries}\nAccept only if every one of these gaps is resolved or invalid on the current tree AND each of {tasks}'s acceptance criteria and must-pass baseline tests pass; if a gap still holds, refuse, or record it again as a residual gap.\n{}",
            round.key,
            disposition_instruction(round)
        ),
        RoundKind::Owned | RoundKind::Expansion => format!(
            "Host round {}: {recorded}; the host routed them to {tasks}.{scope} The gaps (verbatim):\n{gaps}\nThe recording verifiers' summaries (verbatim):\n{summaries}\nFix exactly what they name, keeping every one of {tasks}'s acceptance criteria and must-pass baseline tests passing.",
            round.key
        ),
    }
}

fn asks_for_plan(record: &WorkflowV2CallRecord) -> bool {
    record.call.method == WorkflowV2HostMethod::Checkpoint
        && record.call.options.extra.get(RESIDUAL_GAPS_MARKER) == Some(&Value::Bool(true))
}

/// Whether `call` is the pre-acceptance checkpoint asking for the FIRST
/// pass's plan; the second and third passes' slots are
/// [`super::is_second_pass_slot`] and [`super::is_third_pass_slot`].
pub fn is_residual_slot(call: &WorkflowV2HostCall) -> bool {
    call.method == WorkflowV2HostMethod::Checkpoint
        && call.options.extra.get(RESIDUAL_GAPS_MARKER) == Some(&Value::Bool(true))
        && !super::is_second_pass_slot(call)
        && !super::is_third_pass_slot(call)
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
) -> crate::WorkflowResult<Option<WorkflowV2Result>> {
    let carried = result.data.get(RESIDUAL_GAPS_KEY).is_some();
    if !asks_for_plan(record) && !carried {
        return Ok(None);
    }
    let mut viewed = result.clone();
    if !viewed.data.is_object() {
        viewed.data = json!({});
    }
    if let Some(data) = viewed.data.as_object_mut() {
        data.remove(RESIDUAL_GAPS_KEY);
    }
    if asks_for_plan(record) {
        viewed.data[RESIDUAL_GAPS_KEY] =
            Value::Array(if super::is_second_pass_slot(&record.call) {
                second_pass_view(store, universe, root)
            } else if super::is_third_pass_slot(&record.call) {
                third_pass_view(store, universe, root)?
            } else {
                residual_plan_view(store, universe, root)
            });
    }
    Ok(Some(viewed))
}
