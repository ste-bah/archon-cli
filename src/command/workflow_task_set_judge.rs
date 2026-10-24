//! Batched acceptance judging and policy provenance helpers.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use archon_workflow::WorkflowLlmClient;
use archon_workflow::llm_client_port::WorkflowAgentOutcome;
use archon_workflow::task_set_contract::{
    AcceptanceContract, AcceptancePin, FreezeGateMode, FreezeGateStamp, JudgeDecision,
    content_digest,
};

use crate::command::workflow_gate::{GateFinding, GateId};

#[path = "workflow_task_set_judge_named_files.rs"]
mod named_files;
#[cfg(test)]
use named_files::named_files;
pub(super) use named_files::named_files_digest;
#[cfg(test)]
pub(super) use named_files::named_files_digest_for_list;
pub(super) use named_files::named_files_digest_with_budget;
use named_files::named_files_with_budget;

#[path = "workflow_task_set_judge_rounds.rs"]
mod rounds;
use rounds::judge_prompted_with_roots;

#[cfg(test)]
#[path = "workflow_task_set_judge_prompt_builder_tests.rs"]
mod prompt_builder_tests;

#[path = "workflow_task_set_judge_incomplete.rs"]
mod incomplete;
pub(crate) use incomplete::JudgeIncomplete;
use incomplete::judge_error_class;
/// An incomplete judge from the real producers, for tests of its routing.
#[cfg(test)]
pub(crate) use incomplete::tests::produced as produced_incomplete;

/// The tier alias the whole-set freeze judge is asked for. The client's
/// provider configuration resolves it to the model that judges
/// (`WorkflowLlmClient::resolve_model_alias`), and that resolved model is
/// what a judgment records and a saved repair is keyed by.
pub(crate) const JUDGE_MODEL_ALIAS: &str = "sonnet";

const NAMED_FILES_INSTRUCTION: &str = "A named-file regression guard is valid only when its pass or fail is decided by the existing test or implementation it names, and it fails whenever that test fails. A guard names its deciding file by its literal path relative to the check's working directory. Naming a file or asserting that it exists does not make it the deciding assertion. Judge supplied named_files content as written. These regression-guard rules apply to a check whose deciding test or implementation exists; a named deciding input marked absent is behaviour not implemented yet and is judged by the general rules; a deciding input marked not_supplied has unknown assertions, so the check cannot be accepted as a regression guard on assumed content. Refute a check if an in-bounds state lets its deciding test fail or not run while the check still passes, including when the command discards or masks that test's status, runs it so its failure cannot fail the check, or decides by matching text in a source or test file. A duplicated or relocated implementation may be an in-bounds counterexample when the delivered path uses it while the named test checks an old copy; a speculative counterexample requiring deliberate gaming remains invalid. The host may use a move-aside proof only for named inputs it actually moves; requiring the check to fail when such an input is moved aside is proof of the guard, never a counterexample. Inputs the host does not move, including scripts run through an interpreter and build manifests, do not qualify for that proof.";

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

#[derive(Clone, Debug, Default)]
pub(super) struct JudgeRoots {
    pub project: Option<PathBuf>,
    pub repository: Option<PathBuf>,
    pub task_root: Option<PathBuf>,
    pub repository_commit: Option<String>,
}

#[cfg(test)]
pub(super) fn batched_judge_prompt(contract: &AcceptanceContract) -> Result<String> {
    batched_judge_prompt_with_roots(contract, &JudgeRoots::default())
}

pub(super) fn batched_judge_prompt_with_roots(
    contract: &AcceptanceContract,
    roots: &JudgeRoots,
) -> Result<String> {
    let mut resolved_roots = roots.clone();
    if resolved_roots.repository_commit.is_none() {
        if let Some(repository) = resolved_roots
            .repository
            .as_deref()
            .and_then(|path| path.canonicalize().ok())
        {
            let output = archon_shell::spawn::command("git")
                .arg("-C")
                .arg(repository)
                .args(["rev-parse", "HEAD"])
                .output();
            resolved_roots.repository_commit = output.ok().and_then(|output| {
                output
                    .status
                    .success()
                    .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
            });
        }
    }
    let mut named_file_bytes_remaining =
        archon_workflow::check_source_settle::MAX_JUDGED_SOURCE_BYTES;
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
            let files = named_files_with_budget(
                criterion,
                &resolved_roots,
                &mut named_file_bytes_remaining,
            );
            if !files.is_empty() {
                check["named_files"] = serde_json::json!(files);
            }
            check
        })
        .collect::<Vec<_>>();
    Ok(format!(
        "Adversarially judge every acceptance check below. The toolchain is fixed: the shell, the operating system, environment variables, PATH, and every executable that the repository does not itself build are out of bounds, and a counterexample that stubs, wraps, replaces or shadows any executable, or edits PATH, is invalid and must not refute a check. Everything the implementation produces may vary: the repository's own source and the program it builds from that source, and every file, directory and data artifact under the project root, including any data root the check names. The implementation is fallible, not adversarial: it may be missing, partial, wrong, stale, empty, malformed or hand-placed, its tests may be absent or cover less than the criterion, and its output may be an error message; it does not write source, tests or data whose purpose is to satisfy this check while the criterion is false. {} A counterexample that needs such deliberate gaming, a program that answers true for the check's sake or a test written to pass trivially, is invalid and must not refute a check: under it no check could ever pass. For each id, try to construct such an in-bounds state where the check passes while the criterion is false. Return JSON only as {{\"decisions\":[{{\"id\":\"...\",\"verdict\":\"accepted|refuted\",\"counterexample\":\"...\",\"reason\":\"...\"}}]}} with exactly one decision for every input id and no extra ids. counterexample and reason must each be a non-empty sentence, even when the verdict is accepted: say what the closest passing-but-false state would be, or that none is constructible. Every string must be a single line with newlines escaped as \\n; emit the JSON document alone. Checks: {}",
        NAMED_FILES_INSTRUCTION,
        serde_json::to_string(&checks)?
    ))
}

#[path = "workflow_task_set_judge_partial.rs"]
mod partial;
#[path = "workflow_task_set_judge_reply.rs"]
mod reply;
pub(crate) use partial::PartialReply;
use reply::{Ending, complete_reply, ending};
pub(crate) use reply::{Step, step};

/// Whether `outcome` ended normally (not truncated, not unknown).
pub(super) fn reply_is_complete(outcome: &WorkflowAgentOutcome) -> bool {
    matches!(ending(outcome), Ending::Complete)
}

pub(super) fn apply_judgments(
    contract: &mut AcceptanceContract,
    content: &str,
) -> Result<BTreeSet<String>> {
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
    let extra = actual.difference(&expected).cloned().collect::<Vec<_>>();
    if !extra.is_empty() {
        let missing = expected.difference(&actual).cloned().collect::<Vec<_>>();
        return Err(anyhow!(
            "acceptance judge decision ids do not match the contract: missing={missing:?}, extra={extra:?}; retry the full batch"
        ));
    }
    for criterion in contract
        .acceptance
        .iter_mut()
        .chain(&mut contract.supplementary)
    {
        let Some(decision) = by_id.remove(&criterion.id) else {
            continue;
        };
        criterion.judgment.verdict = decision.verdict;
        criterion.judgment.counterexample = decision.counterexample;
        criterion.judgment.reason = decision.reason;
        criterion.judgment.host_call_id = format!("acceptance-judge-batch:{}", criterion.id);
    }
    Ok(expected.difference(&actual).cloned().collect())
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
const JUDGE_NO_PROGRESS_WINDOW: usize = 3;

/// What the judge is asked when its reply was cut off by the output limit.
pub(crate) const CONTINUE_PROMPT: &str = "Your previous reply was cut off by the output limit. Continue it from exactly the next character: output only the remaining text, repeat nothing already written, and add no preamble, commentary or code fence.";

/// No-progress window on one provider stream, including reasoning and pings.
/// Every observed stream event renews it; a batch has no total time limit.
pub(crate) const JUDGE_TIMEOUT_SECS: u64 = 7_200;

/// Judge `contract` in one batch: a truncated reply is continued, a reply
/// with no usable verdict is re-asked, and a judge that cannot complete is
/// [`JudgeIncomplete`].
#[cfg(test)]
pub(super) async fn judge_contract(
    client: &dyn WorkflowLlmClient,
    contract: AcceptanceContract,
    expected: &BTreeSet<String>,
) -> Result<AcceptanceContract> {
    judge_contract_resumable(client, contract, expected, None).await
}

/// [`judge_contract`], continuing (and saving) the batch's partial reply in
/// `partial` so a retry resumes it instead of asking again (Issue 260).
#[cfg(test)]
pub(super) async fn judge_contract_resumable(
    client: &dyn WorkflowLlmClient,
    contract: AcceptanceContract,
    expected: &BTreeSet<String>,
    partial: Option<&PartialReply<'_>>,
) -> Result<AcceptanceContract> {
    judge_contract_resumable_with_roots(client, contract, expected, partial, &JudgeRoots::default())
        .await
}

#[cfg(test)]
pub(super) async fn judge_contract_resumable_with_roots(
    client: &dyn WorkflowLlmClient,
    contract: AcceptanceContract,
    expected: &BTreeSet<String>,
    partial: Option<&PartialReply<'_>>,
    roots: &JudgeRoots,
) -> Result<AcceptanceContract> {
    judge_batch(
        client,
        contract,
        JUDGE_MODEL_ALIAS,
        |attempt| {
            if expected.is_empty()
                && attempt.acceptance.is_empty()
                && !attempt.supplementary.is_empty()
            {
                // A freeze can reuse every primary acceptance verdict while
                // newly authored supplementary checks still need judging.
                // That subset is not a standalone contract, so validate the
                // judged fields here; the merged full contract is validated
                // by the incremental freeze caller.
                require_judged_prose(attempt)
            } else {
                archon_workflow::task_set_contract::validate_acceptance_structure(
                    attempt, expected, true,
                )
                .map_err(anyhow::Error::new)
            }
        },
        partial,
        roots,
    )
    .await
}

/// Judge an incremental batch. A partial subset cannot establish full
/// contract structure, so validate its decisions and required prose only.
#[cfg(test)]
pub(super) async fn judge_contract_subset_resumable(
    client: &dyn WorkflowLlmClient,
    subset: AcceptanceContract,
    partial: Option<&PartialReply<'_>>,
) -> Result<AcceptanceContract> {
    judge_batch(
        client,
        subset,
        JUDGE_MODEL_ALIAS,
        require_judged_prose,
        partial,
        &JudgeRoots::default(),
    )
    .await
}

pub(super) async fn judge_contract_subset_resumable_with_roots(
    client: &dyn WorkflowLlmClient,
    subset: AcceptanceContract,
    partial: Option<&PartialReply<'_>>,
    roots: &JudgeRoots,
) -> Result<AcceptanceContract> {
    judge_batch(
        client,
        subset,
        JUDGE_MODEL_ALIAS,
        require_judged_prose,
        partial,
        roots,
    )
    .await
}

/// Judge a subset of a contract's checks — the ones being re-authored — in
/// one batch. The subset is not a contract on its own (it may hold no
/// acceptance entry at all), so only each judged entry's recorded fields are
/// required, exactly as the full-contract structure check requires them.
pub(super) async fn judge_entries_with_roots(
    client: &dyn WorkflowLlmClient,
    subset: AcceptanceContract,
    model: &str,
    roots: &JudgeRoots,
) -> Result<AcceptanceContract> {
    judge_batch(client, subset, model, require_judged_prose, None, roots).await
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
    roots: &JudgeRoots,
) -> Result<AcceptanceContract> {
    judge_prompted_with_roots(
        client,
        contract,
        |subset| batched_judge_prompt_with_roots(subset, roots),
        model,
        validate,
        partial,
    )
    .await
}

/// Build the judge's prompt for exactly the entries of `contract`, and
/// apply its decisions to them: a truncated reply is continued (and saved in
/// `partial` when given), a reply with no usable verdict is re-asked, and a
/// judge that cannot complete is [`JudgeIncomplete`].
pub(crate) async fn judge_prompted(
    client: &dyn WorkflowLlmClient,
    contract: AcceptanceContract,
    prompt: impl Fn(&AcceptanceContract) -> Result<String>,
    model: &str,
    validate: impl Fn(&AcceptanceContract) -> Result<()>,
    partial: Option<&PartialReply<'_>>,
) -> Result<AcceptanceContract> {
    judge_prompted_with_roots(client, contract, prompt, model, validate, partial).await
}

fn subset_with_ids(contract: &AcceptanceContract, ids: &BTreeSet<String>) -> AcceptanceContract {
    let mut subset = contract.clone();
    subset.acceptance.retain(|entry| ids.contains(&entry.id));
    subset.supplementary.retain(|entry| ids.contains(&entry.id));
    subset
}

fn subset_without_ids(contract: &AcceptanceContract, ids: &BTreeSet<String>) -> AcceptanceContract {
    let mut subset = contract.clone();
    subset.acceptance.retain(|entry| !ids.contains(&entry.id));
    subset
        .supplementary
        .retain(|entry| !ids.contains(&entry.id));
    subset
}

#[cfg(test)]
#[path = "workflow_acceptance_sampling_tests.rs"]
mod sampling_tests;

#[cfg(test)]
#[path = "workflow_task_set_judge_continuation_tests.rs"]
pub(super) mod continuation_tests;
