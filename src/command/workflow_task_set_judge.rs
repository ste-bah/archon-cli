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

const NAMED_FILES_INSTRUCTION: &str = "When named_files are given, judge the check against what those files actually assert; a counterexample that claims a named file asserts less than it does is invalid. The statement that tests may be absent or cover less than the criterion applies only to behaviour not covered by the given file contents.";

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
            let files = named_files(criterion, &resolved_roots);
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

fn named_files(
    criterion: &archon_workflow::task_set_contract::AcceptanceCriterion,
    roots: &JudgeRoots,
) -> Vec<serde_json::Value> {
    let root = match &criterion.check {
        archon_workflow::task_set_contract::AcceptanceCheck::Command {
            cwd: archon_workflow::task_set_contract::TrustedCwd::RepoRoot,
            ..
        } => roots.repository.as_deref(),
        _ => roots.project.as_deref(),
    };
    let Some(root) = root.and_then(|path| path.canonicalize().ok()) else {
        return Vec::new();
    };
    let project = roots
        .project
        .as_deref()
        .and_then(|path| path.canonicalize().ok())
        .unwrap_or_else(|| root.clone());
    let repository = roots
        .repository
        .as_deref()
        .and_then(|path| path.canonicalize().ok())
        .unwrap_or_else(|| root.clone());
    let task_root = roots
        .task_root
        .as_deref()
        .and_then(|path| path.canonicalize().ok());
    let project_input_excludes = task_root.as_deref().and_then(|task_root| {
        match crate::command::acceptance_scratch_policy::capture(&project, task_root) {
            Ok(Some(binding)) => Some(binding.policy.project_input_excludes),
            Ok(None) => Some(Vec::new()),
            Err(_) => None,
        }
    });
    let task_root_relative = task_root
        .as_deref()
        .and_then(|task_root| task_root.strip_prefix(&project).ok());
    let nested_repository =
        archon_workflow::acceptance_scratch::nested_repository(&project, &repository)
            .filter(|nested| !nested.as_os_str().is_empty());
    let mut inputs = crate::command::workflow_task_set::executability::named_inputs(
        criterion,
        &project,
        &repository,
    );
    inputs.sort();
    inputs.dedup();
    let mut remaining = archon_workflow::check_source_settle::MAX_JUDGED_SOURCE_BYTES;
    inputs
        .into_iter()
        .filter_map(|relative| {
            let path = normalize_named_path(Path::new(&relative))?;
            let absolute = root.join(&path);
            if matches!(
                criterion.check,
                archon_workflow::task_set_contract::AcceptanceCheck::Command {
                    cwd: archon_workflow::task_set_contract::TrustedCwd::RepoRoot,
                    ..
                }
            ) {
                let commit = roots.repository_commit.as_deref()?;
                let stored_path = archon_shell::paths::stored_text(&path);
                let Some(content_bytes) =
                    repository_regular_blob(&repository, commit, &stored_path)
                else {
                    return None;
                };
                let bytes = content_bytes;
                let take = bytes.len().min(remaining);
                let take = utf8_prefix_len(&bytes, take);
                let truncated = take < bytes.len();
                let shown = &bytes[..take];
                remaining -= take;
                let mut content = archon_observability::redaction::redact_secret_values(
                    &String::from_utf8_lossy(shown),
                );
                if truncated {
                    content.push_str("\n[truncated at the existing judged-source byte limit]");
                }
                return Some(
                    serde_json::json!({"path": path.to_string_lossy(), "content": content}),
                );
            } else {
                let canonical = absolute.canonicalize().ok()?;
                if !canonical.starts_with(&root) || !canonical.is_file() {
                    return None;
                }
                let relative = absolute.strip_prefix(&project).ok()?;
                if project_input_excludes.as_ref().is_none_or(|excludes| {
                    archon_workflow::acceptance_scratch::acceptance_input_excluded(
                        relative,
                        excludes,
                        task_root_relative,
                    )
                }) || nested_repository
                    .as_ref()
                    .is_some_and(|nested| relative.starts_with(nested))
                    || relative
                        .components()
                        .any(|component| component.as_os_str() == ".git")
                    || !project_file_is_copied(&project, relative)
                {
                    return None;
                }
            }
            let canonical = absolute.canonicalize().ok()?;
            let bytes = std::fs::read(&canonical).ok()?;
            let take = bytes.len().min(remaining);
            let take = utf8_prefix_len(&bytes, take);
            let truncated = take < bytes.len();
            let shown = &bytes[..take];
            remaining -= take;
            let mut content = archon_observability::redaction::redact_secret_values(
                &String::from_utf8_lossy(shown),
            );
            if truncated {
                content.push_str("\n[truncated at the existing judged-source byte limit]");
            }
            Some(serde_json::json!({
                "path": path.to_string_lossy(),
                "content": content,
            }))
        })
        .collect()
}

/// Digest the exact path/content list the judge prompt will receive.
pub(super) fn named_files_digest(
    criterion: &archon_workflow::task_set_contract::AcceptanceCriterion,
    roots: &JudgeRoots,
) -> String {
    let mut resolved = roots.clone();
    if resolved.repository_commit.is_none() {
        if let Some(repository) = resolved
            .repository
            .as_deref()
            .and_then(|p| p.canonicalize().ok())
        {
            let output = archon_shell::spawn::command("git")
                .arg("-C")
                .arg(repository)
                .args(["rev-parse", "HEAD"])
                .output();
            resolved.repository_commit = output.ok().and_then(|output| {
                output
                    .status
                    .success()
                    .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
            });
        }
    }
    content_digest(
        &serde_json::to_vec(&named_files(criterion, &resolved))
            .expect("named judge files are serializable JSON values"),
    )
}

fn normalize_named_path(path: &Path) -> Option<PathBuf> {
    use std::path::Component;
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(name) => normalized.push(name),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    (!normalized.as_os_str().is_empty()).then_some(normalized)
}

fn utf8_prefix_len(bytes: &[u8], limit: usize) -> usize {
    let limit = limit.min(bytes.len());
    match std::str::from_utf8(&bytes[..limit]) {
        Ok(_) => limit,
        Err(error) if error.error_len().is_none() => error.valid_up_to(),
        Err(_) => limit,
    }
}

fn repository_regular_blob(repository: &Path, commit: &str, path: &str) -> Option<Vec<u8>> {
    let tree_entry = archon_shell::spawn::command("git")
        .arg("-C")
        .arg(repository)
        .args(["ls-tree", "-z", commit, "--", &format!(":(literal){path}")])
        .output()
        .ok()?;
    if !tree_entry.status.success() {
        return None;
    }
    let entry = tree_entry
        .stdout
        .split(|byte| *byte == 0)
        .find_map(|record| {
            let separator = record.iter().position(|byte| *byte == b'\t')?;
            let (metadata, entry_path_with_separator) = record.split_at(separator);
            let entry_path = entry_path_with_separator.get(1..)?;
            (entry_path == path.as_bytes()).then_some(metadata)
        })?;
    // Git regular files are blobs with mode 100644 or 100755. In particular,
    // mode 120000 is a symlink blob and must never be followed or shown.
    if !(entry.starts_with(b"100644 blob ") || entry.starts_with(b"100755 blob ")) {
        return None;
    }
    let output = archon_shell::spawn::command("git")
        .arg("-C")
        .arg(repository)
        .args(["cat-file", "blob", &format!("{commit}:{path}")])
        .output()
        .ok()?;
    output.status.success().then_some(output.stdout)
}

fn project_file_is_copied(project_root: &Path, relative: &Path) -> bool {
    use std::path::Component;
    if relative
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return false;
    }
    let Ok(root) = project_root.canonicalize() else {
        return false;
    };
    let candidate = root.join(relative);
    if candidate.canonicalize().ok().as_deref() != Some(candidate.as_path()) {
        return false;
    }
    let mut current = project_root.to_path_buf();
    let mut components = relative.components().peekable();
    while let Some(std::path::Component::Normal(name)) = components.next() {
        current.push(name);
        let Ok(metadata) = current.symlink_metadata() else {
            return false;
        };
        if metadata.file_type().is_symlink() {
            return false;
        }
        if components.peek().is_some() {
            if !metadata.is_dir() {
                return false;
            }
        } else if !metadata.is_file() {
            return false;
        }
    }
    !relative.as_os_str().is_empty()
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
const JUDGE_NO_PROGRESS_WINDOW: usize = 3;

/// What the judge is asked when its reply was cut off by the output limit.
pub(super) const CONTINUE_PROMPT: &str = "Your previous reply was cut off by the output limit. Continue it from exactly the next character: output only the remaining text, repeat nothing already written, and add no preamble, commentary or code fence.";

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
        "sonnet",
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
pub(super) async fn judge_contract_subset_resumable(
    client: &dyn WorkflowLlmClient,
    subset: AcceptanceContract,
    partial: Option<&PartialReply<'_>>,
) -> Result<AcceptanceContract> {
    judge_batch(
        client,
        subset,
        "sonnet",
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
        "sonnet",
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
    let task = batched_judge_prompt_with_roots(&contract, roots)?;
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
    let mut messages = vec![serde_json::json!({"role":"user","content":task})];
    let mut seen = BTreeSet::new();
    let mut no_progress = 0usize;
    let mut attempts = 0usize;
    let mut last: String;
    loop {
        attempts += 1;
        let content = match complete_reply(client, &messages, model, partial).await? {
            Ok(content) => content,
            Err(unusable) => {
                last = unusable.clone();
                let class = judge_error_class(&unusable);
                if seen.insert(class) {
                    no_progress = 0;
                } else {
                    no_progress += 1;
                }
                if no_progress >= JUDGE_NO_PROGRESS_WINDOW {
                    break;
                }
                // `complete_reply` returns this path only when there is no
                // complete reply to quote (for example, a truncation before
                // JSON starts). Ask afresh; parsed rejections below preserve
                // the actual assistant reply and exact validation error.
                messages = vec![serde_json::json!({"role":"user","content":task})];
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
            Err(error) => {
                last = format!("{error:#}");
                let class = judge_error_class(&last);
                if seen.insert(class) {
                    no_progress = 0;
                } else {
                    no_progress += 1;
                }
                if no_progress >= JUDGE_NO_PROGRESS_WINDOW {
                    break;
                }
                let said = if content.trim().is_empty() {
                    "(an empty reply)"
                } else {
                    &content
                };
                messages = vec![
                    serde_json::json!({"role":"user","content":task}),
                    serde_json::json!({"role":"assistant","content":said}),
                    serde_json::json!({"role":"user","content":format!("Your reply was rejected: {last}\nReply again with the complete JSON document required by the first message.")}),
                ];
            }
        }
    }
    Err(JudgeIncomplete(format!(
        "{attempts} replies; the last {JUDGE_NO_PROGRESS_WINDOW} gave no new first-error class or usable verdict; the last: {last}"
    ))
    .into())
}

fn judge_error_class(error: &str) -> &'static str {
    if error.contains("malformed batched JSON") {
        "malformed_json"
    } else if error.contains("duplicated decision") {
        "duplicate_decision"
    } else if error.contains("decision ids do not match") {
        "decision_ids"
    } else if error.contains("left 'counterexample' empty") {
        "empty_counterexample"
    } else if error.contains("left 'reason' empty") {
        "empty_reason"
    } else if error.contains("unsupported stop reason") || error.contains("no finish reason") {
        "finish_reason"
    } else {
        "invalid_judgment"
    }
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
