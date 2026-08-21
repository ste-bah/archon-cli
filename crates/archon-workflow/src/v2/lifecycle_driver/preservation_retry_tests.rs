// A shape repair rejected for dropping protected fields must be corrected, not
// silently abandoned.
//
// Observed live, twice in one run: the repair re-emitted every route entry
// stripped of `source_residual_gap_ids`, `failed_predicate`, `classification`
// and `canonical_task_ids` — 20 violations across 6 items. The host rejected it
// (correctly), the pre-repair triage stayed authoritative, and the next round
// asked the same reducer the same question and got the same answer. The
// reducer never saw the rejection, so nothing could change.
//
// The corrective re-ask shows it the violations. It is adopted on exactly the
// terms the first attempt faced: preserve identity, and account for more.

use std::sync::Arc;

use serde_json::{Value, json};

use crate::v2::lifecycle_driver::LifecycleEvidence;
use crate::v2::lifecycle_driver::review_test_host::{RecordingHost, driver};

const TASK: &str = "TASK-TDL-001";

/// One non-accepted outcome the triage must account for.
fn failed_outcome() -> Value {
    json!({
        "item_id": "verify-tdl-001",
        "canonical_task_ids": [TASK],
        "status": "needs_review",
        "summary": "provider rows incomplete",
    })
}

/// The pre-repair triage. It carries a fully identified entry, but the entry
/// never names the failed outcome, so accounting reports it unaccounted and the
/// shape repair fires — the live shape of the problem.
fn nested_triage() -> Value {
    json!({
        "status": "needs_review",
        "data": { "items": { "retry_items": [original_entry()] } },
    })
}

/// Identity present, but no `source_item_id` binding it to the outcome.
fn original_entry() -> Value {
    json!({
        "item_id": "retry-tdl-001",
        "canonical_task_ids": [TASK],
        "source_residual_gap_ids": ["gap-1"],
        "failed_predicate": "each provider row addresses all eight dimensions",
        "classification": "retryable_verification_shape_issue",
    })
}

/// What the reducer actually produced live: the outcome now accounted for, and
/// every protected field gone with it.
fn stripped_entry() -> Value {
    json!({
        "item_id": "retry-tdl-001",
        "source_item_id": "verify-tdl-001",
    })
}

/// Accounts for the outcome *and* keeps identity — the only adoptable answer.
fn full_entry() -> Value {
    let mut entry = original_entry();
    entry["source_item_id"] = json!("verify-tdl-001");
    entry
}

fn hoisted(entry: Value) -> Value {
    json!({
        "status": "needs_review",
        "data": { "retry_items": [entry] },
    })
}

/// Replies to the shape repair with `first`, and to the corrective re-ask with
/// `corrected`.
fn host_for(first: Value, corrected: Value) -> Arc<RecordingHost> {
    RecordingHost::new(Box::new(move |_method, call_id| {
        if call_id.ends_with("-preservation-retry") {
            return corrected.clone();
        }
        if call_id.contains("shape-repair") {
            return first.clone();
        }
        json!({ "status": "accepted", "summary": "ok", "data": { "items": [] } })
    }))
}

async fn enforce(host: Arc<RecordingHost>) -> (Value, Arc<RecordingHost>) {
    let driver = driver(host.clone());
    let mut evidence = LifecycleEvidence::default();
    let adopted = driver
        .enforce_triage_accounting(
            "verification-failure-triage-2-3",
            &[failed_outcome()],
            nested_triage(),
            &mut evidence,
        )
        .await
        .expect("accounting");
    (adopted, host)
}

fn retry_items(triage: &Value) -> Vec<Value> {
    triage
        .get("data")
        .and_then(|data| data.get("retry_items"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// The live failure, now recoverable: the first repair strips identity, the
/// corrective re-ask restores it, and the corrected triage is adopted.
#[tokio::test]
async fn a_rejected_repair_is_corrected_and_adopted() {
    let (adopted, host) = enforce(host_for(hoisted(stripped_entry()), hoisted(full_entry()))).await;

    assert!(
        !host
            .ids_starting_with("verification-failure-triage-2-3-shape-repair-1-preservation-retry")
            .is_empty(),
        "a preservation rejection must trigger the corrective re-ask: {:?}",
        host.call_ids()
    );
    let items = retry_items(&adopted);
    assert_eq!(items.len(), 1, "adopted triage: {adopted}");
    assert_eq!(
        items[0].get("failed_predicate"),
        full_entry().get("failed_predicate"),
        "the corrected repair must be adopted with identity intact"
    );
    assert_eq!(
        items[0].get("source_residual_gap_ids"),
        full_entry().get("source_residual_gap_ids")
    );
}

/// A corrective re-ask that strips identity again buys one call and nothing
/// else — the pre-repair triage stays authoritative, exactly as before.
#[tokio::test]
async fn a_second_violation_leaves_the_original_authoritative() {
    let (adopted, host) = enforce(host_for(
        hoisted(stripped_entry()),
        hoisted(stripped_entry()),
    ))
    .await;

    // Count the re-ask itself, not the rejection checkpoint it also emits.
    let reasks = host
        .call_ids()
        .into_iter()
        .filter(|id| id == "verification-failure-triage-2-3-shape-repair-1-preservation-retry")
        .count();
    assert_eq!(
        reasks,
        1,
        "the corrective re-ask is bounded to one attempt: {:?}",
        host.call_ids()
    );
    assert!(
        adopted.pointer("/data/items/retry_items").is_some() || !retry_items(&adopted).is_empty(),
        "the pre-repair triage must survive a second violation: {adopted}"
    );
    for entry in retry_items(&adopted) {
        assert!(
            entry.get("failed_predicate").is_some(),
            "an adopted entry must never be the stripped one: {adopted}"
        );
    }
}
