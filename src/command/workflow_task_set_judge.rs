//! Batched acceptance judging and policy provenance helpers.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

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
            let mut check = serde_json::json!({
                "id": criterion.id,
                "criterion": criterion.criterion,
                "check": criterion.check,
            });
            // The requirement ids the check answers for are part of what it
            // claims: a check covering a requirement it cannot fail on is
            // refuted like one that cannot fail on its criterion.
            if !criterion.covers.is_empty() {
                check["covers"] = serde_json::json!(criterion.covers);
            }
            check
        })
        .collect::<Vec<_>>();
    Ok(format!(
        "Adversarially judge every acceptance check below. The toolchain is fixed: the shell, the operating system, environment variables, PATH, and every executable that the repository does not itself build are out of bounds, and a counterexample that stubs, wraps, replaces or shadows any executable, or edits PATH, is invalid and must not refute a check. Everything the implementation produces may vary: the repository's own source and the program it builds from that source, and every file, directory and data artifact under the project root, including any data root the check names. The implementation is fallible, not adversarial: it may be missing, partial, wrong, stale, empty, malformed or hand-placed, its tests may be absent or cover less than the criterion, and its output may be an error message; it does not write source, tests or data whose purpose is to satisfy this check while the criterion is false. A counterexample that needs such deliberate gaming, a program that answers true for the check's sake or a test written to pass trivially, is invalid and must not refute a check: under it no check could ever pass. For each id, try to construct such an in-bounds state where the check passes while the criterion is false. Return JSON only as {{\"decisions\":[{{\"id\":\"...\",\"verdict\":\"accepted|refuted\",\"counterexample\":\"...\",\"reason\":\"...\"}}]}} with exactly one decision for every input id and no extra ids. counterexample and reason must each be a non-empty sentence, even when the verdict is accepted: say what the closest passing-but-false state would be, or that none is constructible. Every string must be a single line with newlines escaped as \\n; emit the JSON document alone. Checks: {}",
        serde_json::to_string(&checks)?
    ))
}

#[path = "workflow_task_set_judge_partial.rs"]
mod partial;
#[path = "workflow_task_set_judge_reply.rs"]
mod reply;
pub(crate) use partial::PartialReply;
use reply::{Ending, complete_reply, ending};

/// Whether `outcome` ended normally (not truncated, not unknown).
pub(super) fn reply_is_complete(outcome: &WorkflowAgentOutcome) -> bool {
    matches!(ending(outcome), Ending::Complete)
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

/// Consecutive replies that may give no usable verdict (malformed, empty
/// fields, an unknown finish reason) before the judge is incomplete.
///
/// A no-progress bound, not a work budget: each counted reply added nothing
/// a later one could build on. Reaching it never fails the run: the judge
/// is [`JudgeIncomplete`], which a staged freeze reports as resumable, so the
/// host retries the freeze while it makes progress and otherwise pauses.
/// A truncated reply is not counted here: it is continued (Issue 260).
const JUDGE_ATTEMPTS: usize = 3;

/// What the judge is asked when its reply was cut off by the output limit.
pub(super) const CONTINUE_PROMPT: &str = "Your previous reply was cut off by the output limit. Continue it from exactly the next character: output only the remaining text, repeat nothing already written, and add no preamble, commentary or code fence.";

/// No-progress window on one provider stream, including reasoning and pings.
/// Every observed stream event renews it; a batch has no total time limit.
pub(crate) const JUDGE_TIMEOUT_SECS: u64 = 7_200;

/// Judge `contract` in one batch: a truncated reply is continued, a reply
/// with no usable verdict is re-asked, and a judge that cannot complete is
/// [`JudgeIncomplete`].
pub(super) async fn judge_contract(
    client: &dyn WorkflowLlmClient,
    contract: AcceptanceContract,
    expected: &BTreeSet<String>,
) -> Result<AcceptanceContract> {
    judge_contract_resumable(client, contract, expected, None).await
}

/// [`judge_contract`], continuing (and saving) the batch's partial reply in
/// `partial` so a retry resumes it instead of asking again (Issue 260).
pub(super) async fn judge_contract_resumable(
    client: &dyn WorkflowLlmClient,
    contract: AcceptanceContract,
    expected: &BTreeSet<String>,
    partial: Option<&PartialReply<'_>>,
) -> Result<AcceptanceContract> {
    judge_batch(
        client,
        contract,
        "sonnet",
        |attempt| {
            archon_workflow::task_set_contract::validate_acceptance_structure(
                attempt, expected, true,
            )
            .map_err(anyhow::Error::new)
        },
        partial,
    )
    .await
}

/// Judge a subset of a contract's checks — the ones being re-authored — in
/// one batch. The subset is not a contract on its own (it may hold no
/// acceptance entry at all), so only each judged entry's recorded fields are
/// required, exactly as the full-contract structure check requires them.
pub(super) async fn judge_entries(
    client: &dyn WorkflowLlmClient,
    subset: AcceptanceContract,
    model: &str,
) -> Result<AcceptanceContract> {
    judge_batch(client, subset, model, require_judged_prose, None).await
}

/// Every judged entry of `attempt` carries its counterexample and reason.
pub(super) fn require_judged_prose(attempt: &AcceptanceContract) -> Result<()> {
    for entry in attempt.acceptance.iter().chain(&attempt.supplementary) {
        for (field, value) in [
            ("counterexample", entry.judgment.counterexample.as_str()),
            ("reason", entry.judgment.reason.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(anyhow!(
                    "acceptance judge left '{field}' empty for check '{}'; retry the full batch",
                    entry.id
                ));
            }
        }
    }
    Ok(())
}

async fn judge_batch(
    client: &dyn WorkflowLlmClient,
    contract: AcceptanceContract,
    model: &str,
    validate: impl Fn(&AcceptanceContract) -> Result<()>,
    partial: Option<&PartialReply<'_>>,
) -> Result<AcceptanceContract> {
    let task = batched_judge_prompt(&contract)?;
    judge_prompted(client, contract, &task, model, validate, partial).await
}

/// Ask `task` of the judge about exactly the entries of `contract`, and
/// apply its decisions to them: a truncated reply is continued (and saved in
/// `partial` when given), a reply with no usable verdict is re-asked, and a
/// judge that cannot complete is [`JudgeIncomplete`].
pub(super) async fn judge_prompted(
    client: &dyn WorkflowLlmClient,
    contract: AcceptanceContract,
    task: &str,
    model: &str,
    validate: impl Fn(&AcceptanceContract) -> Result<()>,
    partial: Option<&PartialReply<'_>>,
) -> Result<AcceptanceContract> {
    let mut last = String::from("the acceptance judge was never asked");
    for _ in 0..JUDGE_ATTEMPTS {
        let content = match complete_reply(client, task, model, partial).await? {
            Ok(content) => content,
            Err(unusable) => {
                last = unusable;
                continue;
            }
        };
        // A batch that parses but leaves a verdict field empty is the same kind
        // of slip as one that will not parse, so it is re-asked rather than
        // ending the freeze: the judged shape is what makes a batch usable.
        let mut attempt = contract.clone();
        let judged = apply_judgments(&mut attempt, &content).and_then(|()| validate(&attempt));
        // An unusable reply is spent with its credit. A usable reply keeps
        // its continuation until JudgeStore durably saves the verdicts.
        if judged.is_err()
            && let Some(partial) = partial
        {
            partial.spent();
        }
        match judged {
            Ok(()) => {
                for entry in attempt
                    .acceptance
                    .iter_mut()
                    .chain(&mut attempt.supplementary)
                {
                    entry.judgment.sampling = Some(serde_json::json!({
                        "temperature": 0.0, "model": client.resolve_model_alias(model),
                        "provider": client.provider_id(),
                    }));
                }
                return Ok(attempt);
            }
            Err(error) => last = format!("{error:#}"),
        }
    }
    Err(JudgeIncomplete(format!(
        "{JUDGE_ATTEMPTS} consecutive replies gave no usable verdict; the last: {last}"
    ))
    .into())
}

/// The judge could not complete its batch (Issue 260): an operational,
/// resumable outcome, never a verdict and never a candidate defect.
#[derive(Debug)]
pub(crate) struct JudgeIncomplete(pub(crate) String);

impl JudgeIncomplete {
    /// The incomplete judge `error` carries, if any.
    pub(crate) fn caused(error: &anyhow::Error) -> Option<&Self> {
        error.chain().find_map(|cause| cause.downcast_ref::<Self>())
    }
}

impl std::fmt::Display for JudgeIncomplete {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "operational: acceptance judge incomplete, resumable: {}",
            self.0
        )
    }
}

impl std::error::Error for JudgeIncomplete {}

#[cfg(test)]
#[path = "workflow_acceptance_sampling_tests.rs"]
mod sampling_tests;

#[cfg(test)]
#[path = "workflow_task_set_judge_continuation_tests.rs"]
pub(super) mod continuation_tests;
