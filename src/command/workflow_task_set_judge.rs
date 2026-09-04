//! Batched acceptance judging and policy provenance helpers.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use archon_workflow::WorkflowLlmClient;
use archon_workflow::llm_client_port::WorkflowAgentOutcome;
use archon_workflow::task_set_contract::{
    AcceptanceContract, AcceptancePin, FreezeGateMode, FreezeGateStamp, JudgeDecision,
    content_digest,
};

use crate::command::workflow_gate::{GateFinding, GateId};

#[derive(Debug, serde::Deserialize)]
struct BatchedJudgeResponse {
    decisions: Vec<JudgeResponse>,
}

#[derive(Debug, serde::Deserialize)]
struct JudgeResponse {
    id: String,
    verdict: JudgeDecision,
    counterexample: String,
    reason: String,
}

pub(super) fn batched_judge_prompt(contract: &AcceptanceContract) -> Result<String> {
    let checks = contract
        .acceptance
        .iter()
        .chain(&contract.supplementary)
        .map(|criterion| {
            serde_json::json!({
                "id": criterion.id,
                "criterion": criterion.criterion,
                "check": criterion.check,
            })
        })
        .collect::<Vec<_>>();
    Ok(format!(
        "Adversarially judge every acceptance check below. The toolchain is fixed: the shell, the operating system, environment variables, PATH, and every executable that the repository does not itself build are out of bounds, and a counterexample that stubs, wraps, replaces or shadows any executable, or edits PATH, is invalid and must not refute a check. Everything the implementation produces may vary: the repository's own source and the program it builds from that source, and every file, directory and data artifact under the project root, including any data root the check names. For each id, try to construct such an in-bounds state where the check passes while the criterion is false. Return JSON only as {{\"decisions\":[{{\"id\":\"...\",\"verdict\":\"accepted|refuted\",\"counterexample\":\"...\",\"reason\":\"...\"}}]}} with exactly one decision for every input id and no extra ids. counterexample and reason must each be a non-empty sentence, even when the verdict is accepted: say what the closest passing-but-false state would be, or that none is constructible. Every string must be a single line with newlines escaped as \\n; emit the JSON document alone. Checks: {}",
        serde_json::to_string(&checks)?
    ))
}

pub(super) fn require_complete_judge_response(outcome: &WorkflowAgentOutcome) -> Result<()> {
    match outcome.stop_reason.as_deref() {
        Some("end_turn" | "stop" | "completed") => Ok(()),
        Some("max_tokens" | "length") => Err(anyhow!(
            "acceptance judge response was truncated by stop reason '{}'; raise the output budget or reduce the batch, then retry — partial JSON is never repaired",
            outcome.stop_reason.as_deref().unwrap_or_default()
        )),
        Some(reason) => Err(anyhow!(
            "acceptance judge ended with unsupported stop reason '{reason}'; retry only after the provider can complete the batch normally"
        )),
        None => Err(anyhow!(
            "acceptance judge returned no finish reason; refusing to parse possibly truncated JSON"
        )),
    }
}

pub(super) fn apply_judgments(contract: &mut AcceptanceContract, content: &str) -> Result<()> {
    // The judge is a model too, and models package documents in prose and code
    // fences. The host decides what counts as the reply here exactly as it does
    // for an authored candidate, so one provider's habits cannot fail a freeze
    // that the judge actually answered.
    let document =
        crate::command::workflow_freeze_candidate::candidate_document(content.trim().as_bytes());
    let response: BatchedJudgeResponse = serde_json::from_slice(&document).with_context(|| {
        let fault = std::str::from_utf8(&document)
            .ok()
            .and_then(archon_workflow::describe_json_fault)
            .unwrap_or_default();
        format!("acceptance judge returned malformed batched JSON ({fault}); retry the full batch")
    })?;
    let expected: BTreeSet<_> = contract
        .acceptance
        .iter()
        .chain(&contract.supplementary)
        .map(|criterion| criterion.id.clone())
        .collect();
    let mut by_id = BTreeMap::new();
    for decision in response.decisions {
        let id = decision.id.clone();
        if by_id.insert(id.clone(), decision).is_some() {
            return Err(anyhow!(
                "acceptance judge duplicated decision '{id}'; retry the full batch"
            ));
        }
    }
    let actual: BTreeSet<_> = by_id.keys().cloned().collect();
    if actual != expected {
        let missing = expected.difference(&actual).cloned().collect::<Vec<_>>();
        let extra = actual.difference(&expected).cloned().collect::<Vec<_>>();
        return Err(anyhow!(
            "acceptance judge decision ids do not match the contract: missing={missing:?}, extra={extra:?}; retry the full batch"
        ));
    }
    for criterion in contract
        .acceptance
        .iter_mut()
        .chain(&mut contract.supplementary)
    {
        let decision = by_id.remove(&criterion.id).expect("exact id set checked");
        criterion.judgment.verdict = decision.verdict;
        criterion.judgment.counterexample = decision.counterexample;
        criterion.judgment.reason = decision.reason;
        criterion.judgment.host_call_id = format!("acceptance-judge-batch:{}", criterion.id);
    }
    Ok(())
}

pub(super) fn gate_stamp(mode: FreezeGateMode, findings: &[GateFinding]) -> FreezeGateStamp {
    let ordered = findings
        .iter()
        .map(|finding| {
            serde_json::json!({
                "gate_id": finding.gate_id.as_str(),
                "finding": finding.text,
                "subject": finding.subject,
                "source_path": finding.source_path,
            })
        })
        .collect::<Vec<_>>();
    let bytes = serde_json::to_vec(&ordered).expect("gate findings serialize");
    FreezeGateStamp {
        mode,
        finding_count: findings.len(),
        findings_digest: content_digest(&bytes),
        binary_commit: env!("ARCHON_GIT_HASH").into(),
        evaluated_at: chrono::Utc::now().to_rfc3339(),
    }
}

pub(super) fn predecessor_findings(
    pin: &AcceptancePin,
    pin_path: &Path,
    findings: &mut Vec<GateFinding>,
) {
    if pin.acceptance_gate.finding_count == 0 {
        return;
    }
    findings.push(GateFinding::new(
        GateId::FreezeSkeleton,
        format!(
            "predecessor acceptance freeze was minted in {:?} mode with {} policy finding(s); re-freeze under enforce and resolve every named finding before continuing",
            pin.acceptance_gate.mode,
            pin.acceptance_gate.finding_count
        ),
        "acceptance-freeze",
        Some(pin_path.to_path_buf()),
        archon_workflow::RemediationScope::InheritedPredecessor,
    ));
}

#[cfg(test)]
#[path = "workflow_task_set_judge_tests.rs"]
mod workflow_task_set_judge_tests;

/// How many times a malformed batch may be re-asked.
///
/// A malformed reply is a formatting slip the same prompt often gets right on a
/// second pass. A truncated one is not: the budget that cut it off has not
/// changed, so asking again only spends another call. Only the first is retried.
const JUDGE_ATTEMPTS: usize = 3;

/// Shared with the freeze path so both speak of one budget.
const JUDGE_TIMEOUT_SECS: u64 = 1_500;

/// Judge `contract` in one batch, re-asking only when the reply malforms.
pub(super) async fn judge_contract(
    client: &dyn WorkflowLlmClient,
    contract: AcceptanceContract,
    expected: &BTreeSet<String>,
) -> Result<AcceptanceContract> {
    let task = batched_judge_prompt(&contract)?;
    let mut last = anyhow!("acceptance judge was never asked");
    for _ in 0..JUDGE_ATTEMPTS {
        let outcome = tokio::time::timeout(
            Duration::from_secs(JUDGE_TIMEOUT_SECS),
            client.send_message(
                vec![serde_json::json!({ "role": "user", "content": task.clone() })],
                Vec::new(),
                Vec::new(),
                "sonnet",
            ),
        )
        .await
        .map_err(|_| {
            anyhow!(
                "acceptance judge timed out after {JUDGE_TIMEOUT_SECS}s; retry the freeze when the provider can complete the full batch"
            )
        })?
        .map_err(anyhow::Error::new)?;
        // A truncated answer ends it here: re-asking cannot widen the budget
        // that cut it off, and the partial JSON is never repaired.
        require_complete_judge_response(&outcome)?;
        // A batch that parses but leaves a verdict field empty is the same kind
        // of slip as one that will not parse, so it is re-asked rather than
        // ending the freeze: the judged shape is what makes a batch usable.
        let mut attempt = contract.clone();
        match apply_judgments(&mut attempt, &outcome.content).and_then(|()| {
            archon_workflow::task_set_contract::validate_acceptance_structure(
                &attempt, expected, true,
            )
            .map_err(anyhow::Error::new)
        }) {
            Ok(()) => return Ok(attempt),
            Err(error) => last = error,
        }
    }
    Err(last)
}
