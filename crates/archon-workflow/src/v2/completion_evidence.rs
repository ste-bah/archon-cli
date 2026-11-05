//! Durable per-task completion evidence, attributed from a branch outcome.
//!
//! The completion ledger credits a task as done only against evidence that
//! survives the run: `completed = noop ∪ (implementation ∩ verification)`. The
//! evidence is minted here, from the branch outcome that produced it, and
//! written onto [`WorkflowV2BranchOutcome::completion_evidence`] so it persists
//! with the outcome rather than being re-derived later from a result whose
//! shape the agent controls.
//!
//! Attribution is keyed on the CALL ID, not on the result body: the call id is
//! the runtime's own, so an agent cannot mint itself a completion by shaping
//! its output. A call whose id matches no known scheme mints nothing.
//!
//! Nothing is minted for a branch that did not finish cleanly, or that names no
//! task, or that offers no evidence — a credit with no evidence behind it is
//! exactly what the ledger exists to prevent.

use crate::generated_contract::{
    canonical_task_ids_from_generated_value, evidence_refs_from_generated_value,
};
use crate::v2::host_api::WorkflowV2HostCall;
use crate::v2::result::{
    WorkflowV2CommandStatus, WorkflowV2EvidenceKind, WorkflowV2Result, WorkflowV2Status,
    WorkflowV2TaskCoverageStatus,
};
use crate::v2::result_store::{
    WorkflowV2TaskCompletionEvidence, WorkflowV2TaskCompletionEvidenceKind,
};
use crate::v2::scheduler::WorkflowV2BranchOutcome;

/// Version stamped into a focused-verification call's input and read back off
/// its evidence, so evidence minted under an older contract is distinguishable
/// from evidence minted under this one.
pub const FOCUSED_VERIFICATION_EVIDENCE_CONTRACT_VERSION: &str = "focused-verification-evidence-v2";

/// Mint completion evidence onto `outcome` for the task(s) it speaks for.
pub fn attach_completion_evidence_for_call(
    call: &WorkflowV2HostCall,
    outcome: &mut WorkflowV2BranchOutcome,
) {
    let Some(kind) = task_completion_evidence_kind(&call.id) else {
        return;
    };
    if !matches!(
        outcome.status,
        WorkflowV2Status::Accepted | WorkflowV2Status::Noop
    ) {
        return;
    }
    let Some(result) = outcome.result.as_ref() else {
        return;
    };
    let task_ids = canonical_task_ids_from_result(result);
    let evidence_refs = evidence_summaries_from_result(result);
    if task_ids.is_empty() || evidence_refs.is_empty() {
        return;
    }
    let artifact_paths = result
        .artifacts
        .iter()
        .map(|artifact| artifact.path.trim().to_string())
        .filter(|path| !path.is_empty())
        .collect::<Vec<_>>();
    let command_refs = result
        .commands_run
        .iter()
        .filter(|command| command.status == WorkflowV2CommandStatus::Succeeded)
        .map(|command| command.command.trim().to_string())
        .filter(|command| !command.is_empty())
        .collect::<Vec<_>>();
    let source_call_id = string_value(result.data.get("source_call_id"))
        .or_else(|| string_value(result.data.get("sourceCallId")));
    let source_item_id = string_value(result.data.get("source_item_id"))
        .or_else(|| string_value(result.data.get("sourceItemId")));
    let mut evidence = Vec::new();
    for task_id in task_ids {
        let mut item = WorkflowV2TaskCompletionEvidence::new(
            task_id,
            kind.clone(),
            call.id.clone(),
            outcome.item_id.clone(),
            outcome.status,
        );
        item.source_call_id = source_call_id.clone();
        item.source_item_id = source_item_id.clone();
        if matches!(
            kind,
            WorkflowV2TaskCompletionEvidenceKind::FocusedVerification
        ) {
            item.source_fingerprint =
                Some(FOCUSED_VERIFICATION_EVIDENCE_CONTRACT_VERSION.to_string());
        }
        item.evidence_refs = evidence_refs.clone();
        item.artifact_paths = artifact_paths.clone();
        item.command_refs = command_refs.clone();
        item.item_input_hash = outcome.item_input_hash.clone();
        evidence.push(item);
    }
    outcome.completion_evidence = evidence;
}

/// Which kind of completion evidence a call id mints, if any.
pub fn task_completion_evidence_kind(
    call_id: &str,
) -> Option<WorkflowV2TaskCompletionEvidenceKind> {
    if call_id.starts_with("noop-proof-verification-")
        || call_id.starts_with("noop-proof-reverification-")
    {
        return Some(WorkflowV2TaskCompletionEvidenceKind::VerifiedNoop);
    }
    if call_id.starts_with("verification-wave-") || call_id.starts_with("review-verification-wave-")
    {
        return Some(WorkflowV2TaskCompletionEvidenceKind::FocusedVerification);
    }
    if call_id.starts_with("implementation-wave-")
        || call_id.starts_with("remediation-wave-")
        || call_id.starts_with("review-remediation-wave-")
        // v3 authored-script implement/remediate calls, symmetric with the
        // verify calls above (which already match `verification-wave-`). Without
        // these, an accepted v3 implementation stamps no ImplementationCandidate
        // evidence, the implementation credit set stays empty for a pure-v3 run,
        // and `completed = noop ∪ (implementation ∩ verification)` credits no
        // task — so resume/restart can never skip completed work. The prefixes
        // are the runtime's own v3 call-id scheme, not PRD-specific.
        || call_id.starts_with("implement-task-")
        || call_id.starts_with("remediate-task-")
    {
        return Some(WorkflowV2TaskCompletionEvidenceKind::ImplementationCandidate);
    }
    None
}

/// Every canonical task id a result speaks for, unioned across the generated
/// contract's own extraction, the several spellings agents use, and accepted or
/// no-op task coverage.
pub fn canonical_task_ids_from_result(result: &WorkflowV2Result) -> Vec<String> {
    let mut ids = canonical_task_ids_from_generated_value(&result.data, None);
    ids.extend(
        string_array(result.data.get("canonical_task_ids"))
            .into_iter()
            .chain(string_array(result.data.get("canonicalTaskIds")))
            .chain(string_array(result.data.get("canonical_task_id")))
            .chain(string_array(result.data.get("canonicalTaskId")))
            .chain(string_array(result.data.get("task_ids")))
            .chain(string_array(result.data.get("taskIds")))
            .chain(string_array(result.data.get("task_id")))
            .collect::<Vec<_>>(),
    );
    ids.extend(result.task_coverage.iter().filter_map(|coverage| {
        matches!(
            coverage.status,
            WorkflowV2TaskCoverageStatus::Accepted | WorkflowV2TaskCoverageStatus::Noop
        )
        .then(|| coverage.task_id.trim().to_string())
        .filter(|task_id| !task_id.is_empty())
    }));
    sorted_unique(ids)
}

/// Concrete evidence summaries a result offers.
///
/// An accepted or no-op result is held to a higher bar: only implementation and
/// test evidence, succeeded commands, and accepted or no-op coverage count.
/// Anything else is still finding its way to review, so its gaps and raw
/// evidence array count too.
pub fn evidence_summaries_from_result(result: &WorkflowV2Result) -> Vec<String> {
    let accepted_or_noop = matches!(
        result.status,
        WorkflowV2Status::Accepted | WorkflowV2Status::Noop
    );
    let mut evidence = result
        .evidence
        .iter()
        .filter(|item| {
            !accepted_or_noop
                || matches!(
                    item.kind,
                    WorkflowV2EvidenceKind::Implementation | WorkflowV2EvidenceKind::Test
                )
        })
        .map(|item| item.summary.trim().to_string())
        .filter(|summary| !summary.is_empty())
        .collect::<Vec<_>>();
    for coverage in &result.task_coverage {
        if accepted_or_noop
            && !matches!(
                coverage.status,
                WorkflowV2TaskCoverageStatus::Accepted | WorkflowV2TaskCoverageStatus::Noop
            )
        {
            continue;
        }
        evidence.extend(
            coverage
                .evidence
                .iter()
                .map(|item| item.summary.trim().to_string())
                .filter(|summary| !summary.is_empty()),
        );
    }
    evidence.extend(
        result
            .commands_run
            .iter()
            .filter(|command| {
                !accepted_or_noop || command.status == WorkflowV2CommandStatus::Succeeded
            })
            .map(|command| command.command.trim().to_string())
            .filter(|command| !command.is_empty()),
    );
    evidence.extend(
        result
            .files_changed
            .iter()
            .map(|file| file.path.trim().to_string())
            .filter(|path| !path.is_empty()),
    );
    evidence.extend(
        result
            .artifacts
            .iter()
            .map(|artifact| artifact.path.trim().to_string())
            .filter(|path| !path.is_empty()),
    );
    evidence.extend(evidence_refs_from_generated_value(&result.data));
    if !accepted_or_noop {
        evidence.extend(
            result
                .residual_gaps
                .iter()
                .map(|gap| gap.description.trim().to_string())
                .filter(|description| !description.is_empty()),
        );
        evidence.extend(string_array(result.data.get("evidence")));
    }
    sorted_unique(evidence)
}

/// Strings from a JSON array, or from a comma-separated string.
pub fn string_array(value: Option<&serde_json::Value>) -> Vec<String> {
    match value {
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .filter_map(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect(),
        Some(serde_json::Value::String(value)) => value
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

/// A non-empty trimmed string from a JSON value.
pub fn string_value(value: Option<&serde_json::Value>) -> Option<String> {
    value
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// Trim, drop empties, de-duplicate, and order.
pub fn sorted_unique(values: Vec<String>) -> Vec<String> {
    use std::collections::BTreeSet;

    values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}
