//! Whether a frozen phase may switch to its verify-frozen path on resume, and
//! the exact reason, so an operator can read every decision in the dry run.

use anyhow::{Context, Result, anyhow};
use archon_workflow::{
    PublicationReceiptV1, WorkflowV2CallRecord, WorkflowV2HostMethod, WorkflowV2Status,
};

/// One phase's decision. `verify` is true only when every safety condition
/// held; `reason` names the record and the condition that decided it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OverrideDecision {
    pub(crate) verify: bool,
    pub(crate) reason: String,
}

impl OverrideDecision {
    pub(crate) fn keep(reason: impl Into<String>) -> Self {
        Self {
            verify: false,
            reason: reason.into(),
        }
    }

    fn verify(reason: impl Into<String>) -> Self {
        Self {
            verify: true,
            reason: reason.into(),
        }
    }
}

pub(super) fn host_command_records(
    store: &archon_workflow::WorkflowStore,
    run_id: &str,
) -> Result<Vec<WorkflowV2CallRecord>> {
    let mut records: Vec<_> =
        archon_workflow::WorkflowV2ResultStore::new(store.run_dir(run_id).join("v2"))
            .load_call_records()?
            .into_iter()
            .filter(|record| record.call.method == WorkflowV2HostMethod::HostCommand)
            .collect();
    records.sort_by(attempt_order);
    Ok(records)
}

fn capability_of(record: &WorkflowV2CallRecord) -> Option<&str> {
    record
        .call
        .options
        .host_command
        .as_ref()
        .map(|request| request.command_id.as_str())
}

fn committed_receipt(record: &WorkflowV2CallRecord) -> bool {
    record.result.data["publicationReceipt"].is_object()
}

/// A verdict-less interruption does not replace the last judged attempt.
/// `records` are this run's host-command records in attempt order.
pub(super) fn freeze_override_decision(
    records: &[WorkflowV2CallRecord],
    capability: &str,
) -> Result<OverrideDecision> {
    let attempts: Vec<(usize, &WorkflowV2CallRecord)> = records
        .iter()
        .enumerate()
        .filter(|(_, record)| capability_of(record) == Some(capability))
        .collect();
    let Some(judged) = attempts.iter().rposition(|(_, record)| has_verdict(record)) else {
        return Ok(OverrideDecision::keep(format!(
            "no {capability} attempt with a gate verdict ({} attempt(s) recorded)",
            attempts.len()
        )));
    };
    let (position, record) = attempts[judged];
    let id = &record.call.id;
    if let Some((_, later)) = attempts[judged + 1..]
        .iter()
        .find(|(_, later)| committed_receipt(later))
    {
        return Ok(OverrideDecision::keep(format!(
            "later attempt {} published after judged attempt {id}",
            later.call.id
        )));
    }
    if !freeze_record_status_allows_override(record.status, record.invalidated_by.is_some()) {
        return Ok(OverrideDecision::keep(format!(
            "judged attempt {id} status={} invalidated={}",
            status_text(record.status),
            record.invalidated_by.is_some()
        )));
    }
    let data = &record.result.data;
    if data["postcondition"]["satisfied"] != true {
        return Ok(OverrideDecision::keep(format!(
            "judged attempt {id} postcondition not satisfied"
        )));
    }
    let Some(receipt_value) = data
        .get("publicationReceipt")
        .filter(|value| value.is_object())
    else {
        return Ok(OverrideDecision::keep(format!(
            "judged attempt {id} has no publication receipt"
        )));
    };
    let receipt: PublicationReceiptV1 = serde_json::from_value(receipt_value.clone())
        .context("parsing frozen-chain publication receipt")?;
    if receipt.command_id != capability || receipt.call_id != record.call.id {
        return Ok(OverrideDecision::keep(format!(
            "judged attempt {id} receipt names command {} call {}",
            receipt.command_id, receipt.call_id
        )));
    }
    // Only a receipt bound to its own record vouches for a later write; any
    // other leaves that write unaccounted for, which keeps the phase.
    let mut successors: Vec<PublicationReceiptV1> = Vec::new();
    for later in records[position + 1..]
        .iter()
        .filter(|later| later.invalidated_by.is_none() && committed_receipt(later))
    {
        let next: PublicationReceiptV1 =
            serde_json::from_value(later.result.data["publicationReceipt"].clone())
                .with_context(|| format!("parsing publication receipt of {}", later.call.id))?;
        if next.schema_version == archon_workflow::PUBLICATION_RECEIPT_SCHEMA_VERSION
            && next.call_id == later.call.id
            && capability_of(later) == Some(next.command_id.as_str())
        {
            successors.push(next);
        }
    }
    let carried = match receipt_accounts_for_live(&receipt, &successors)? {
        Ok(carried) => carried,
        Err(mismatch) => {
            return Ok(OverrideDecision::keep(format!(
                "judged attempt {id} receipt does not match disk: {mismatch}"
            )));
        }
    };
    let mut decision = freeze_outcome_decision(data, capability, id)?;
    if !carried.is_empty() {
        decision.reason.push_str(&format!(
            "; rewritten by later publications: {}",
            carried.join(", ")
        ));
    }
    Ok(decision)
}

/// Every receipt entry must be on disk as published, or reached from it by a
/// chain of later committed publications in this run, each by a later
/// pipeline stage and each replacing exactly the bytes the previous link left
/// (`prior_blake3`). A shared file such as the acceptance pin is legitimately
/// extended by the next freeze; any other write leaves the verdict stale. `Ok` names
/// each entry a later publication rewrote, and by whom.
pub(super) fn receipt_accounts_for_live(
    receipt: &PublicationReceiptV1,
    successors: &[PublicationReceiptV1],
) -> Result<std::result::Result<Vec<String>, String>> {
    let mut carried = Vec::new();
    for entry in &receipt.entries {
        let (mut digest, mut length, mut writer) = (
            entry.blake3.clone(),
            entry.byte_len,
            receipt.call_id.as_str(),
        );
        for successor in successors {
            for link in successor
                .entries
                .iter()
                .filter(|link| link.destination_path == entry.destination_path)
            {
                let stage = super::super::workflow_host_command_exec::pipeline_stage;
                if stage(&successor.command_id) <= stage(&receipt.command_id) {
                    return Ok(Err(format!(
                        "{} was replaced by {} {}, which is not a later pipeline stage than {}",
                        entry.relative_path,
                        successor.command_id,
                        successor.call_id,
                        receipt.command_id
                    )));
                }
                if link.prior_blake3.as_deref() != Some(digest.as_str()) {
                    return Ok(Err(format!(
                        "{} was replaced by {} {} from {} but {writer} left {digest}",
                        entry.relative_path,
                        successor.command_id,
                        successor.call_id,
                        link.prior_blake3.as_deref().unwrap_or("nothing")
                    )));
                }
                (digest, length, writer) = (
                    link.blake3.clone(),
                    link.byte_len,
                    successor.call_id.as_str(),
                );
            }
        }
        let path = std::path::Path::new(&entry.destination_path);
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Err(format!("{} is missing", entry.relative_path)));
            }
            Err(error) => {
                return Err(error).with_context(|| format!("reading {}", path.display()));
            }
        };
        let live = archon_workflow::task_set_contract::content_digest(&bytes);
        if bytes.len() as u64 != length || live != digest {
            return Ok(Err(format!(
                "{} digests to {live} ({} bytes) but the last recorded publication {writer} left {digest} ({length} bytes)",
                entry.relative_path,
                bytes.len()
            )));
        }
        if writer != receipt.call_id {
            carried.push(format!("{} by {writer}", entry.relative_path));
        }
    }
    Ok(Ok(carried))
}

pub(super) fn freeze_record_status_allows_override(
    status: WorkflowV2Status,
    invalidated: bool,
) -> bool {
    !invalidated && matches!(status, WorkflowV2Status::Accepted)
}

/// A clean judged outcome flips only when this build cannot reuse it.
pub(super) fn freeze_outcome_decision(
    outcome: &serde_json::Value,
    capability: &str,
    id: &str,
) -> Result<OverrideDecision> {
    let findings = outcome
        .get("gateEnvelope")
        .and_then(|envelope| envelope.get("policy_findings"))
        .and_then(serde_json::Value::as_array);
    match findings {
        None => {
            return Ok(OverrideDecision::keep(format!(
                "judged attempt {id} gate envelope carries no policy_findings"
            )));
        }
        Some(findings) if !findings.is_empty() => {
            return Ok(OverrideDecision::keep(format!(
                "judged attempt {id} has {} open finding(s)",
                findings.len()
            )));
        }
        Some(_) => {}
    }
    let current_version = super::super::workflow_host_command_logic::versions()
        .get(capability)
        .copied()
        .ok_or_else(|| anyhow!("host command capability '{capability}' has no logic version"))?;
    let (current_digest, build_bound) = super::super::workflow_host_command_logic::digests()
        .remove(capability)
        .ok_or_else(|| anyhow!("host command capability '{capability}' has no logic digest"))?;
    let recorded_version = outcome
        .get("logicVersion")
        .and_then(serde_json::Value::as_u64);
    let version_stale = recorded_version != Some(u64::from(current_version));
    let digest_stale = outcome
        .get("logicDigest")
        .and_then(serde_json::Value::as_str)
        .is_none_or(|digest| digest != current_digest);
    let build_stale = build_bound
        && outcome
            .get("logicBuild")
            .and_then(serde_json::Value::as_str)
            .is_none_or(|build| build != super::super::workflow_host_command_logic::THIS_BUILD);
    let versions = format!(
        "logicVersion recorded={} current={current_version} digest_stale={digest_stale} build_stale={build_stale}",
        recorded_version.map_or_else(|| "none".to_string(), |version| version.to_string())
    );
    Ok(if version_stale || digest_stale || build_stale {
        OverrideDecision::verify(format!(
            "judged attempt {id} accepted with 0 findings, published, matches disk; not reusable by this build ({versions})"
        ))
    } else {
        OverrideDecision::keep(format!(
            "judged attempt {id} is reusable by this build ({versions})"
        ))
    })
}

fn status_text(status: WorkflowV2Status) -> String {
    serde_json::to_value(status)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| format!("{status:?}"))
}

fn has_verdict(record: &WorkflowV2CallRecord) -> bool {
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

#[cfg(test)]
mod tests {
    #[test]
    fn shared_output_is_carried_only_by_a_later_pipeline_stage() {
        let temp = tempfile::tempdir().unwrap();
        let pin = temp.path().join("pin.json");
        let digest = |bytes: &[u8]| archon_workflow::task_set_contract::content_digest(bytes);
        let receipt = |command: &str, bytes: &[u8], prior: Option<&[u8]>| {
            serde_json::from_value::<archon_workflow::PublicationReceiptV1>(serde_json::json!({
                "schema_version": 1, "call_id": format!("{command}-call"), "command_id": command,
                "committed_at": "2026-10-10T10:00:00Z",
                "entries": [{
                    "relative_path": "acceptance-pin.json", "destination_path": pin,
                    "byte_len": bytes.len(), "blake3": digest(bytes),
                    "prior_blake3": prior.map(digest),
                }]
            }))
            .unwrap()
        };
        let judged = receipt("freeze-skeleton", b"first", None);
        std::fs::write(&pin, b"second").unwrap();
        let account = super::receipt_accounts_for_live;
        let later = receipt("land-task-body", b"second", Some(b"first"));
        assert_eq!(
            account(&judged, std::slice::from_ref(&later)).unwrap(),
            Ok(vec![
                "acceptance-pin.json by land-task-body-call".to_string()
            ])
        );
        let upstream = receipt("freeze-acceptance", b"second", Some(b"first"));
        assert!(
            account(&judged, &[upstream])
                .unwrap()
                .unwrap_err()
                .contains("not a later pipeline stage")
        );
        assert!(
            account(&judged, &[])
                .unwrap()
                .unwrap_err()
                .contains("digests to")
        );
    }
}
