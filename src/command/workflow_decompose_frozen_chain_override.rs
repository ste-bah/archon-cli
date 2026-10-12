use anyhow::{Context, Result};
use archon_workflow::{WorkflowV2CallRecord, WorkflowV2HostMethod, WorkflowV2ResultStore};

use super::{freeze_outcome_requires_rejudging, freeze_record_status_allows_override};

/// A verdict-less interruption does not replace the last judged attempt.
pub(super) fn freeze_needs_rejudging(
    store: &archon_workflow::WorkflowStore,
    run_id: &str,
    capability: &str,
) -> Result<bool> {
    let records =
        WorkflowV2ResultStore::new(store.run_dir(run_id).join("v2")).load_call_records()?;
    let mut attempts: Vec<_> = records
        .iter()
        .filter(|record| {
            record.call.method == WorkflowV2HostMethod::HostCommand
                && record
                    .call
                    .options
                    .host_command
                    .as_ref()
                    .is_some_and(|request| request.command_id == capability)
        })
        .collect();
    attempts.sort_by(|left, right| attempt_order(left, right));
    let Some(index) = attempts.iter().rposition(|record| has_verdict(record)) else {
        return Ok(false);
    };
    let record = attempts[index];
    if attempts[index + 1..]
        .iter()
        .any(|later| later.result.data["publicationReceipt"].is_object())
    {
        return Ok(false);
    }
    if !freeze_record_status_allows_override(record.status, record.invalidated_by.is_some()) {
        return Ok(false);
    }
    let data = &record.result.data;
    if data["postcondition"]["satisfied"] != true {
        return Ok(false);
    }
    let Some(receipt_value) = data
        .get("publicationReceipt")
        .filter(|value| value.is_object())
    else {
        return Ok(false);
    };
    let receipt: archon_workflow::PublicationReceiptV1 =
        serde_json::from_value(receipt_value.clone())
            .context("parsing frozen-chain publication receipt")?;
    if receipt.command_id != capability
        || receipt.call_id != record.call.id
        || !super::super::workflow_host_command_postcondition::receipt_matches_live(Some(&receipt))?
    {
        return Ok(false);
    }
    freeze_outcome_requires_rejudging(data, capability)
}

fn has_verdict(record: &WorkflowV2CallRecord) -> bool {
    use archon_workflow::WorkflowV2Status;
    record.invalidated_by.is_none()
        && matches!(
            record.status,
            WorkflowV2Status::Accepted | WorkflowV2Status::NeedsReview | WorkflowV2Status::Noop
        )
        && record
            .result
            .data
            .get("gateEnvelope")
            .is_some_and(|envelope| {
                envelope.is_object()
                    && envelope
                        .get("operational_error")
                        .is_none_or(serde_json::Value::is_null)
                    && envelope
                        .get("policy_findings")
                        .is_some_and(serde_json::Value::is_array)
            })
}

fn attempt_order(left: &WorkflowV2CallRecord, right: &WorkflowV2CallRecord) -> std::cmp::Ordering {
    let timestamp_order = || {
        chrono::DateTime::parse_from_rfc3339(&left.started_at)
            .ok()
            .cmp(&chrono::DateTime::parse_from_rfc3339(&right.started_at).ok())
    };
    match (left.admission_sequence, right.admission_sequence) {
        (Some(left), Some(right)) => left.cmp(&right).then_with(timestamp_order),
        _ => timestamp_order().then_with(|| left.admission_sequence.cmp(&right.admission_sequence)),
    }
    .then_with(|| left.call.id.cmp(&right.call.id))
    .then_with(|| left.attempt.cmp(&right.attempt))
}
