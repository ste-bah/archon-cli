//! The branch half of `script::resume_drift`: a review remediation write is a
//! dynamic fan-out, which the host never replays whole (the repository audit
//! admits cached write credit only per item), so its reuse is decided here,
//! per branch, after the ordinary rule in the parent module refused.

use super::*;
use crate::v2::result_store::WorkflowV2CallRecord;
use crate::v2::reuse_identity::{DRIFT_IDENTITIES_KEY, REUSE_INPUT_HASH_KEY, reuse_input_hash};
use crate::v2::scheduler::BranchFailureKind;
use crate::v2::script::is_reusable_status;
use crate::v2::script::resume_drift::{
    drift_candidates, ordinal_token, rebase_text, rebase_value, restarted,
    same_remediation_contract, superseded_remediation_record,
};
use crate::v2::script::resume_verdict::verdict_vouches_for_session_fix;

#[path = "branch_cache_nochange.rs"]
mod nochange;
use nochange::stands_unchanged;

#[path = "branch_cache_lineage.rs"]
mod lineage;
pub use lineage::{forget_fix_lineage, replayed_fix};
pub(super) use lineage::{label_last_written, note_fix_lineage};

/// Whether a branch reused from `source`'s record may stand: a remediation
/// verdict only while this session's fix was replayed from the fix that
/// verdict judged (`script::resume_verdict`), and only when that record is
/// provably the execution whose answer is replayed -- nothing of the
/// verdict's label was written after it (`label_written`, read before this
/// session saved any). Anything else may.
pub(super) fn verdict_allows(
    v2_store: &WorkflowV2ResultStore,
    item: &WorkflowV2FanoutItem,
    source: &str,
    records: Option<&[WorkflowV2CallRecord]>,
    label_written: Option<std::time::SystemTime>,
) -> bool {
    if !crate::v2::script::resume_verdict::is_remediation_verdict(&item.call) {
        return true;
    }
    records.is_some_and(|records| {
        records
            .iter()
            .find(|record| record.call.id == source)
            .is_some_and(|record| {
                lineage::wrote_nothing_after(v2_store, record, label_written)
                    && verdict_vouches_for_session_fix(record, records, v2_store)
            })
    })
}

/// Whether `call_id`'s own record belongs to another round than `item`: the
/// id was reused under a label cut short, so its outcome answers a
/// different question.
pub(super) fn foreign_round(
    call_id: &str,
    item: &WorkflowV2FanoutItem,
    records: &[WorkflowV2CallRecord],
) -> bool {
    records
        .iter()
        .find(|record| record.call.id == call_id)
        .is_some_and(|record| !same_remediation_contract(&record.call, &item.call))
}

/// History is a verdict the script acted on, never credit: an accepted or
/// no-op record answers only through the reusable rule, with its receipt
/// and evidence checks.
fn history_eligible(outcome: &WorkflowV2BranchOutcome) -> bool {
    answered(outcome)
        && !matches!(
            outcome.status,
            WorkflowV2Status::Accepted | WorkflowV2Status::Noop
        )
}

/// A recorded branch that answered: a result that agrees with its status and
/// validates, and no execution or safety failure. A branch the host got no
/// answer from carries no result or an execution failure.
fn answered(outcome: &WorkflowV2BranchOutcome) -> bool {
    outcome.status != WorkflowV2Status::Cancelled
        && outcome.error.is_none()
        && !matches!(
            outcome.failure_kind,
            Some(BranchFailureKind::Execution | BranchFailureKind::Safety)
        )
        && outcome
            .result
            .as_ref()
            .is_some_and(|result| result.status == outcome.status && result.validate().is_ok())
}

/// A sibling write counts only with the host's own receipt that its patch
/// went through apply (`patch_landed` is stamped when the worktree patch is
/// captured, before the wave applies it) and the tree still holding it --
/// unless it positively recorded that it changed nothing. A missing or
/// unreadable marker is not "nothing landed".
fn landing_receipt_holds(
    v2_store: &WorkflowV2ResultStore,
    item: &WorkflowV2FanoutItem,
    sibling_call: &str,
    sibling: &WorkflowV2BranchOutcome,
) -> bool {
    let manifest = manifest_record(v2_store, sibling_call, &sibling.item_id);
    item.call.write_mode.is_none()
        || stands_unchanged(v2_store, sibling_call, sibling, manifest.as_ref())
        || (manifest_landed(v2_store, sibling_call, &sibling.item_id)
            && tree_holds_landing(v2_store, sibling_call, sibling, item))
}

/// Whether the canonical tree still holds what `call_id`'s recorded
/// `outcome` landed, as the run left it: every path its apply manifest
/// recorded holds the state the run's last landing there left -- this one,
/// or later landings of this run provably after it
/// (`landing::landing_holds`, Issue-108). A replayed remediation write --
/// drifted or under its own id -- stands only then; a path changed outside
/// the run's own landings means the recorded answer (and the verdict that
/// judged it) is not what the repository says, and the fix runs again. A
/// `skipped_ignored` manifest is a receipt with nothing in the git tree to
/// check; a project artifact any landing materialized must still stand
/// where it is verified (`materialized::landing_copies_hold`). A manifest
/// that failed, conflicted or never applied is no receipt: a credited
/// answer with one runs again, history landed nothing to check.
///
/// A write that positively recorded no change ([`stands_unchanged`]) stands
/// with or without a manifest: the tree is whatever later stages left, and
/// the script reads its answer as a round that landed no patch, never as a
/// resolution. Any other credited record without a manifest -- capture
/// failed, a record older than the receipts, one refiled from a sibling
/// whose manifest is filed under the sibling's id -- is ambiguous and runs
/// again (a refiled one is then re-judged against the sibling's manifest by
/// the drift rule). A read-only branch lands nothing.
pub(super) fn tree_holds_landing(
    v2_store: &WorkflowV2ResultStore,
    call_id: &str,
    outcome: &WorkflowV2BranchOutcome,
    item: &WorkflowV2FanoutItem,
) -> bool {
    if item.call.write_mode.is_none() {
        return true;
    }
    let item_id = outcome.item_id.as_str();
    let manifest = manifest_record(v2_store, call_id, item_id);
    if stands_unchanged(v2_store, call_id, outcome, manifest.as_ref()) {
        return true;
    }
    // Issue-113: an ignored project artifact the landing placed where it is
    // verified must still be what the run's last copy there left. Checked
    // first: it needs no repository root.
    let run_root = v2_store.root().parent().unwrap_or(v2_store.root());
    if let Err(reason) =
        super::materialized::landing_copies_hold(run_root, call_id, item_id, manifest.as_ref())
    {
        eprintln!("remediation replay: {call_id}/{item_id} does not stand: {reason}");
        return false;
    }
    let Some(root) = item
        .input
        .get("item")
        .and_then(|item| item.get("target_repository_root"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|root| !root.is_empty())
    else {
        eprintln!(
            "remediation replay: {call_id}/{item_id} has no repository root; tree not checked"
        );
        return true;
    };
    // A credited answer needs a receipt that its work went through apply;
    // history is no credit, and what it recorded never landed.
    let credited = matches!(
        outcome.status,
        WorkflowV2Status::Accepted | WorkflowV2Status::Noop
    );
    let landed = manifest.filter(|manifest| {
        matches!(
            manifest.status,
            ManifestStatus::Applied
                | ManifestStatus::IdempotentNoop
                | ManifestStatus::SkippedIgnored
        )
    });
    let Some(manifest) = landed else {
        eprintln!(
            "remediation replay: {call_id}/{item_id} has no applied manifest; {}",
            if credited {
                "no receipt and no record that it changed nothing, so it runs again"
            } else {
                "not a credited answer, nothing landed to check"
            }
        );
        return !credited;
    };
    if manifest.status == ManifestStatus::SkippedIgnored {
        // Ignored deliverables never enter the tree (`patch_sidecar`), and a
        // copy at their path there is the one the worktree was seeded from,
        // not the fix's: the tree has nothing of this fix to check.
        eprintln!(
            "remediation replay: {call_id}/{item_id} manifest is SkippedIgnored; nothing landed in the git tree to check"
        );
        return true;
    }
    match super::landing::landing_holds(Path::new(root), &manifest) {
        Ok(()) => true,
        Err(reason) => {
            eprintln!("remediation replay: {call_id}/{item_id} does not stand: {reason}");
            false
        }
    }
}

/// Carry, on every review-remediation branch, its AUTHORED input's identity
/// rebased to each same-label sibling's ordinal. Must run right after
/// `stamp_reuse_input_hash`, before any other stamp: the host then writes
/// non-volatile content into the input (contract and task-declared targets
/// into `item.target_files`, dependency and contract context), and a rebase
/// of the stamped input can never reproduce the authored identity a sibling
/// was recorded under. Live on wf-0ddadd81 that sent every shifted write
/// back to a coder.
pub fn stamp_drift_identities(
    branches: &mut [WorkflowV2FanoutItem],
    call_id: &str,
    v2_store: &WorkflowV2ResultStore,
) -> WorkflowResult<()> {
    if !branches
        .iter()
        .any(|branch| is_remediation_call(&branch.call))
    {
        return Ok(());
    }
    let Some((token, own_ordinal)) = ordinal_token(call_id) else {
        return Ok(());
    };
    let records = v2_store.load_call_records()?;
    let candidates = drift_candidates(call_id, &records, |_| false);
    for branch in branches.iter_mut() {
        if !is_remediation_call(&branch.call) {
            continue;
        }
        let identities: serde_json::Map<String, serde_json::Value> = candidates
            .iter()
            .filter_map(|record| {
                // Only a sibling that could answer: reusable (credit), or a
                // superseded rejected round (history, no credit).
                let credit = is_reusable_status(record.status);
                if !credit && !superseded_remediation_record(record, &records) {
                    return None;
                }
                let (_, sibling_ordinal) = ordinal_token(&record.call.id)?;
                let rebased = rebase_value(&branch.input, token, own_ordinal, sibling_ordinal);
                Some((
                    record.call.id.clone(),
                    serde_json::json!({ "identity": reuse_input_hash(&rebased), "credit": credit }),
                ))
            })
            .collect();
        if !identities.is_empty()
            && let Some(object) = branch.input.as_object_mut()
        {
            object.insert(
                DRIFT_IDENTITIES_KEY.to_string(),
                serde_json::Value::Object(identities),
            );
        }
    }
    Ok(())
}

/// Whether `stamp_drift_identities` found a sibling this branch may be
/// credited from -- the one case worth an audit refresh, which the run's
/// unexpected-change allowance pays for. History grants no credit.
pub fn has_drift_identities(branch: &WorkflowV2FanoutItem) -> bool {
    branch
        .input
        .get(DRIFT_IDENTITIES_KEY)
        .and_then(serde_json::Value::as_object)
        .is_some_and(|identities| {
            identities
                .values()
                .any(|entry| entry.get("credit").and_then(serde_json::Value::as_bool) == Some(true))
        })
}

/// `item` as it would have been built under the `to` ordinal, for the
/// sibling `sibling_call`. Its identity is the authored one rebased when the
/// write path carried it; otherwise (a read-only branch, stamped nothing
/// non-volatile) it is recomputed from the rebased input.
fn rebased_item(
    item: &WorkflowV2FanoutItem,
    sibling_call: &str,
    token: &str,
    from: &str,
    to: &str,
) -> WorkflowV2FanoutItem {
    let carried = item
        .input
        .get(DRIFT_IDENTITIES_KEY)
        .and_then(|identities| identities.get(sibling_call))
        .and_then(|entry| entry.get("identity"))
        .cloned();
    let mut rebased = item.clone();
    rebased.id = rebase_text(&item.id, token, from, to);
    rebased.call.id = rebase_text(&item.call.id, token, from, to);
    rebased.input = rebase_value(&item.input, token, from, to);
    if let Some(object) = rebased.input.as_object_mut() {
        match carried {
            Some(identity) => {
                object.insert(REUSE_INPUT_HASH_KEY.to_string(), identity);
            }
            None => {
                object.remove(REUSE_INPUT_HASH_KEY);
            }
        }
    }
    rebased
}

/// The sibling's outcome refiled as this branch's own: its branch and
/// evidence ids moved to this call, under this item's identity. Paths it
/// recorded (worktree, patch, manifest) keep naming the sibling's files,
/// which are the ones that exist.
fn refiled(
    outcome: &WorkflowV2BranchOutcome,
    item: &WorkflowV2FanoutItem,
    call_id: &str,
    sibling_call: &str,
) -> WorkflowV2BranchOutcome {
    let identity = reuse_identity(item);
    let mut refiled = outcome.clone();
    if let Some(data) = refiled
        .result
        .as_mut()
        .and_then(|result| result.data.as_object_mut())
    {
        for key in ["branch_id", "item_id"] {
            if data.get(key).and_then(serde_json::Value::as_str) == Some(outcome.item_id.as_str()) {
                data.insert(key.to_string(), serde_json::Value::String(item.id.clone()));
            }
        }
    }
    for evidence in &mut refiled.completion_evidence {
        if evidence.call_id == sibling_call {
            evidence.call_id = call_id.to_string();
        }
        if evidence.item_id == outcome.item_id {
            evidence.item_id = item.id.clone();
        }
        if evidence.item_input_hash.is_some() {
            evidence.item_input_hash = Some(identity.clone());
        }
    }
    refiled.item_id = item.id.clone();
    refiled.item_input_hash = Some(identity);
    refiled
}

/// The outcome a remediation branch is reused as when the ordinary rule
/// refused its own record, or `None` to dispatch it. In order: the same work
/// recorded under another ordinal and reusable; this branch's own answer when
/// a later round has superseded its call; a superseded sibling's answer.
/// Every match is content-keyed -- the item's authored identity, rebased to
/// the sibling's ordinal, must be the identity the sibling was recorded under
/// -- and contract-keyed: the item identity holds no round (the prompt is the
/// same every round), so the sibling's remediation contract must equal this
/// call's.
pub(super) fn remediation_outcome(
    v2_store: &WorkflowV2ResultStore,
    call_id: &str,
    item: &WorkflowV2FanoutItem,
    current: Option<&WorkflowV2BranchOutcome>,
    records: &[WorkflowV2CallRecord],
) -> WorkflowResult<Option<(WorkflowV2BranchOutcome, String)>> {
    let Some((token, own_ordinal)) = ordinal_token(call_id) else {
        return Ok(None);
    };
    if restarted(call_id, records) {
        return Ok(None);
    }
    let candidates = drift_candidates(call_id, records, |id| v2_store.in_session(id));
    let mut history = None;
    for record in candidates {
        let Some((_, sibling_ordinal)) = ordinal_token(&record.call.id) else {
            continue;
        };
        if !same_remediation_contract(&record.call, &item.call) {
            continue;
        }
        let rebased = rebased_item(item, &record.call.id, token, own_ordinal, sibling_ordinal);
        let Some(sibling) = v2_store.load_branch_outcome(&record.call.id, &rebased.id)? else {
            continue;
        };
        if reusable_branch_outcome_for_item(&record.call.id, &sibling, &rebased)
            && landing_receipt_holds(v2_store, item, &record.call.id, &sibling)
        {
            v2_store.note_session_call(&record.call.id);
            return Ok(Some((
                refiled(&sibling, item, call_id, &record.call.id),
                record.call.id.clone(),
            )));
        }
        let matches = sibling
            .item_input_hash
            .as_deref()
            .is_some_and(|recorded| recorded_hash_matches(recorded, &rebased));
        if history.is_none()
            && matches
            && history_eligible(&sibling)
            && superseded_remediation_record(record, records)
            && tree_holds_landing(v2_store, &record.call.id, &sibling, item)
        {
            history = Some((record.call.id.clone(), sibling));
        }
    }
    let own_history = current.filter(|outcome| {
        history_eligible(outcome)
            && !v2_store.in_session(call_id)
            && !foreign_round(call_id, item, records)
            && outcome
                .item_input_hash
                .as_deref()
                .is_some_and(|recorded| recorded_hash_matches(recorded, item))
            && records
                .iter()
                .find(|record| record.call.id == call_id)
                .is_some_and(|record| superseded_remediation_record(record, records))
    });
    if let Some(outcome) = own_history {
        return Ok(Some((outcome.clone(), call_id.to_string())));
    }
    Ok(history.map(|(sibling_call, sibling)| {
        v2_store.note_session_call(&sibling_call);
        let outcome = refiled(&sibling, item, call_id, &sibling_call);
        (outcome, sibling_call)
    }))
}
