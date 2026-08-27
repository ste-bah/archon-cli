//! Batched acceptance judging and policy provenance helpers.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result, anyhow};
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
        "Adversarially judge every acceptance check below. For each id, try to construct a filesystem state where the check passes while the criterion is false. Return JSON only as {{\"decisions\":[{{\"id\":\"...\",\"verdict\":\"accepted|refuted\",\"counterexample\":\"...\",\"reason\":\"...\"}}]}} with exactly one decision for every input id and no extra ids. Checks: {}",
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
    let response: BatchedJudgeResponse = serde_json::from_str(content.trim())
        .context("acceptance judge returned malformed batched JSON; retry the full batch")?;
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
    ));
}
