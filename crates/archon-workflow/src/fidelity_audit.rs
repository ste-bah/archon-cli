//! Obligation fidelity: is a PRD obligation *necessarily* true once every task
//! that claims it passes its own acceptance?
//!
//! # Why a set comparison was not enough
//!
//! Coverage asks whether some task *claims* each obligation. Observed live: a
//! PRD obliged native ingestion into a shared registry, four tasks claimed the
//! criterion, and each scoped itself to "library-level tests against a
//! temporary root" — every task passed, the registry held nothing, the claim
//! was honoured to the letter and the obligation was never delivered. Whether
//! a task's own allowances (temporary roots, pending statuses, minimum counts
//! of zero, optional lanes, fail-closed gaps) leave the obligation standing is
//! a reading of prose, so it is asked of a model — but asked one typed
//! question, answered in one strict shape, checked here before anything
//! downstream believes it.
//!
//! This module carries the prompt, the verdict shape, and the parser. It knows
//! no domain: it reads only the PRD's obligation text, the task files' own
//! text and the set's frozen skeleton, and every rule below is about shape
//! and provenance, never content.
//!
//! # Why the skeleton travels with the question (Issue-45)
//!
//! Ordering and ownership between tasks are frozen before any body is
//! written (`depends_on`, `blocks`, `implements`, `deliverable_contracts`),
//! and the body gate rejects an edit to them. Asked without the skeleton, the
//! critic refuted an obligation on the ground that the chain "only blocks B,
//! leaving C unordered" while the frozen graph had C depend on B — a verdict
//! no author could act on. The per-body audit runs before sibling bodies
//! exist, so the skeleton is the only place those facts can be read from.

use serde::{Deserialize, Serialize};

use crate::task_skeleton::TaskSkeleton;

/// Longest `reason` kept — a verdict is a pointer to a loophole, not an
/// essay, and a finding that quotes it must stay readable in a log line. A
/// longer reason is cut here, never refused (Issue-42).
pub const MAX_REASON_CHARS: usize = 400;
/// Longest `quoted_task_text` kept, verbatim from the weakest task.
pub const MAX_QUOTE_CHARS: usize = 300;

/// One PRD obligation with the exact text the PRD states for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClaimedObligation {
    pub id: String,
    pub text: String,
}

/// One task that claims an obligation, with its full file text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimingTask {
    pub task_id: String,
    pub text: String,
}

/// The frozen skeleton as the critic reads it: one line per task in the set,
/// or the statement that the set has none. Built once per audit and shared by
/// every cluster; part of each cluster's digest, so a re-frozen skeleton
/// re-asks every cluster.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkeletonSummary {
    text: String,
}

/// One skeleton task on one line: the ids and paths the skeleton stores,
/// nothing derived. Line-per-task JSON keeps the section compact and leaves
/// the critic no formatting to interpret.
#[derive(Serialize)]
struct SkeletonLine<'a> {
    task_id: &'a str,
    file_name: &'a str,
    depends_on: Vec<SkeletonDependency<'a>>,
    blocks: &'a [String],
    implements: &'a [String],
    deliverable_contracts: Vec<SkeletonDeliverable<'a>>,
}

#[derive(Serialize)]
struct SkeletonDependency<'a> {
    task_id: &'a str,
    consumes: Vec<&'a str>,
    ordering_only: bool,
}

#[derive(Serialize)]
struct SkeletonDeliverable<'a> {
    kind: &'a str,
    artifact_path: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    registry_path: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    instance_source_path: Option<&'a str>,
}

impl SkeletonSummary {
    /// Every task of a frozen skeleton, in skeleton order.
    pub fn from_skeleton(skeleton: &TaskSkeleton) -> Self {
        let mut text =
            String::from("FROZEN SKELETON (every task in the set, one JSON object per line):\n");
        for task in &skeleton.tasks {
            let line = SkeletonLine {
                task_id: &task.task_id,
                file_name: &task.file_name,
                depends_on: task
                    .depends_on
                    .iter()
                    .map(|dependency| SkeletonDependency {
                        task_id: &dependency.task_id,
                        consumes: dependency
                            .consumes
                            .iter()
                            .map(|artifact| artifact.artifact_path.as_str())
                            .collect(),
                        ordering_only: dependency.ordering_only,
                    })
                    .collect(),
                blocks: &task.blocks,
                implements: &task.implements,
                deliverable_contracts: task
                    .deliverable_contracts
                    .iter()
                    .map(|contract| SkeletonDeliverable {
                        kind: &contract.kind,
                        artifact_path: &contract.artifact_path,
                        registry_path: contract.registry_path.as_deref(),
                        instance_source_path: contract.instance_source_path.as_deref(),
                    })
                    .collect(),
            };
            text.push_str(&serde_json::to_string(&line).unwrap_or_default());
            text.push('\n');
        }
        Self { text }
    }

    /// A set with no `task-skeleton.json` beside its tasks: legacy sets
    /// linted with `workflow lint --tasks` are still audited, and the critic
    /// is told that only the task texts establish ordering and ownership.
    pub fn absent() -> Self {
        Self {
            text: String::from(
                "FROZEN SKELETON: this task set has no frozen skeleton; only the task texts below establish inter-task ordering and ownership.\n",
            ),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }
}

/// The typed answer for one obligation.
///
/// `weakest_task_id` and `quoted_task_text` default to empty when omitted: a
/// true verdict has nothing to quote, and a critic that leaves the two fields
/// out of a true verdict has still answered. A false verdict is checked for
/// both by [`parse_fidelity_response`], so the default never softens the
/// verdict that matters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FidelityVerdict {
    pub obligation_id: String,
    pub necessarily_true: bool,
    #[serde(default)]
    pub weakest_task_id: String,
    pub reason: String,
    #[serde(default)]
    pub quoted_task_text: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FidelityResponse {
    verdicts: Vec<FidelityVerdict>,
}

/// An operator's recorded decision to freeze despite a false verdict.
///
/// Recorded verbatim in the freeze pin so the waiver is auditable beside the
/// stamp it overrides: who waived is the operator running the command, when is
/// stamped, and the reason is the operator's own words, unedited.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObligationWaiver {
    pub obligation_id: String,
    pub reason: String,
    pub waived_at: String,
    pub binary_commit: String,
}

/// The one question, asked over a cluster of obligations that share exactly
/// the same claiming tasks so each task's text travels once per cluster. The
/// skeleton section sits between the obligations and the task texts.
pub fn fidelity_prompt(
    obligations: &[ClaimedObligation],
    tasks: &[ClaimingTask],
    skeleton: &SkeletonSummary,
) -> String {
    let mut prompt = format!(
        "You are auditing whether a task decomposition is faithful to its PRD. Below are PRD obligations, each with the exact text the PRD states, followed by the FROZEN SKELETON of the whole task set, followed by the FULL text of every task file that claims to implement them or that a claiming task names as owning part of the result. Assume every listed task passes its own acceptance criteria and focused tests exactly as written, including every allowance the task text grants itself (temporary roots, pending/deferred statuses, fail-closed residual gaps, minimum counts of zero, optional lanes). For each obligation: is the PRD obligation then necessarily true? Answer strictly: it is necessarily true only when no reading of the task texts lets every task pass while the obligation stays false in the project the PRD describes; a task that satisfies itself somewhere other than where the PRD requires the result, or that permits the result to be absent, deferred or empty, does not make the obligation true. Return JSON only as {{\"verdicts\":[{{\"obligation_id\":\"...\",\"necessarily_true\":true,\"weakest_task_id\":\"...\",\"reason\":\"...\",\"quoted_task_text\":\"...\"}}]}} with exactly one verdict for every obligation id and no extra ids or fields. reason: one line of at most {MAX_REASON_CHARS} characters saying why, never empty — for a true verdict it names what in the task text obliges the result. When necessarily_true is false, weakest_task_id names the listed task whose allowance grants the loophole and quoted_task_text is an excerpt of at most {MAX_QUOTE_CHARS} characters copied verbatim from that task's text, exactly as it appears; when necessarily_true is true, both are empty strings. Every string is a single line with newlines escaped as \\n; emit the JSON document alone.\n\nThe FROZEN SKELETON lists every task in the set with its frozen depends_on, blocks, implements and deliverable_contracts. Inter-task ordering and result ownership are FACTS established by the frozen skeleton, not allowances in task prose: depends_on is transitive, so a task runs after everything its dependencies depend on, and a task's implements and deliverable_contracts say what it owns. A task listed in the skeleton whose full text is not included below is not yet written; it will be audited when it is written and again at the set gate, so its absence is never by itself a ground to refute an obligation. Judge the allowances in the task texts that ARE included, against the ordering and ownership the skeleton establishes, and never refute an obligation on ordering or ownership grounds the skeleton already guarantees. When the set has no frozen skeleton the section says so, and only the task texts establish ordering and ownership.\n\nREPOSITORY PATH CLAIMS: the task set was decomposed against one code repository at one recorded base commit, and a task text that says a backticked repository path exists, or does not exist and will be created, is making a claim the host checks deterministically against that repository and refuses when it is wrong. You are not shown the repository. Never refute or accept an obligation on your own belief about whether a named file exists; take a task's existence claims as the host-checked facts they are and judge only whether, given them, the task text obliges the result where the PRD requires it.\n\nObligations: {}\n\n{}",
        serde_json::to_string(obligations).unwrap_or_default(),
        skeleton.as_str()
    );
    for task in tasks {
        prompt.push_str(&format!(
            "\n===== BEGIN TASK {} =====\n{}\n===== END TASK {} =====\n",
            task.task_id, task.text, task.task_id
        ));
    }
    prompt
}

/// Parse a reply strictly against the cluster it answers.
///
/// Every defect of provenance is an error, never a default: a missing verdict
/// is not a pass, an unknown obligation id is not ignored, a quote that does
/// not appear in the named task is not a quote. The caller re-asks once and
/// then treats the failure as operational — a verdict the host cannot check is
/// a verdict the host does not have. Length alone is not a defect: an
/// over-long reason or quote is cut to its limit and kept (see
/// [`check_verdict`]).
pub fn parse_fidelity_response(
    document: &str,
    obligations: &[ClaimedObligation],
    tasks: &[ClaimingTask],
) -> Result<Vec<FidelityVerdict>, String> {
    let response: FidelityResponse = serde_json::from_str(document.trim())
        .map_err(|error| format!("fidelity reply is not the verdict document: {error}"))?;
    let mut by_id = std::collections::BTreeMap::new();
    for verdict in response.verdicts {
        if by_id
            .insert(verdict.obligation_id.clone(), verdict)
            .is_some()
        {
            return Err("fidelity reply repeats an obligation id".to_string());
        }
    }
    let expected: std::collections::BTreeSet<&str> =
        obligations.iter().map(|o| o.id.as_str()).collect();
    let actual: std::collections::BTreeSet<&str> = by_id.keys().map(String::as_str).collect();
    if expected != actual {
        return Err(format!(
            "fidelity reply verdict ids do not match the cluster: missing={:?}, extra={:?}",
            expected.difference(&actual).collect::<Vec<_>>(),
            actual.difference(&expected).collect::<Vec<_>>()
        ));
    }
    let mut verdicts = Vec::with_capacity(by_id.len());
    for obligation in obligations {
        let mut verdict = by_id.remove(&obligation.id).expect("id set checked");
        check_verdict(&mut verdict, tasks)?;
        verdicts.push(verdict);
    }
    Ok(verdicts)
}

/// Check one verdict's provenance against its cluster, cutting an over-long
/// `reason` or `quoted_task_text` to its limit first.
///
/// Issue-42: a cluster failed the gate twice with "reason longer than 400
/// characters" although both its verdicts were true — the critic had answered
/// the question and merely said too much, and the gate turned a verbose pass
/// into an operational failure. Length is now a cut, not a refusal: the reason
/// keeps its first [`MAX_REASON_CHARS`] characters plus `…`; the quote keeps
/// its first [`MAX_QUOTE_CHARS`] characters and nothing more, so the verbatim
/// check below runs on exactly the text the finding will print. Provenance is
/// still refused outright: a weakest task outside the cluster, a false verdict
/// that names no weakest task or quotes nothing, or a quote the named task
/// does not contain.
fn check_verdict(verdict: &mut FidelityVerdict, tasks: &[ClaimingTask]) -> Result<(), String> {
    let id = &verdict.obligation_id;
    if verdict.reason.trim().is_empty() {
        return Err(format!("verdict for {id} has an empty reason"));
    }
    if verdict.reason.chars().count() > MAX_REASON_CHARS {
        let mut cut: String = verdict.reason.chars().take(MAX_REASON_CHARS).collect();
        cut.push('…');
        verdict.reason = cut;
    }
    if verdict.quoted_task_text.chars().count() > MAX_QUOTE_CHARS {
        // A cut that lands after the backslash of an escaped newline would
        // leave a stray backslash no task text contains; a shorter prefix
        // of a verbatim excerpt is still verbatim, so it is dropped.
        let cut: String = verdict
            .quoted_task_text
            .chars()
            .take(MAX_QUOTE_CHARS)
            .collect();
        verdict.quoted_task_text = cut.trim_end_matches('\\').to_string();
    }
    let weakest = tasks
        .iter()
        .find(|task| task.task_id == verdict.weakest_task_id);
    if verdict.necessarily_true {
        if !verdict.weakest_task_id.is_empty() && weakest.is_none() {
            return Err(format!(
                "verdict for {id} names weakest task '{}' which is not in the audited cluster",
                verdict.weakest_task_id
            ));
        }
        return Ok(());
    }
    let Some(weakest) = weakest else {
        return Err(format!(
            "false verdict for {id} names weakest task '{}' which is not in the audited cluster",
            verdict.weakest_task_id
        ));
    };
    let quote = collapse_whitespace(&verdict.quoted_task_text);
    if quote.is_empty() {
        return Err(format!(
            "false verdict for {id} quotes nothing from the task"
        ));
    }
    if !collapse_whitespace(&weakest.text).contains(&quote) {
        return Err(format!(
            "false verdict for {id} quotes text that does not appear verbatim in task '{}'",
            weakest.task_id
        ));
    }
    Ok(())
}

/// Whitespace-insensitive comparison: a model that re-wraps a line has still
/// quoted it, while one that paraphrases has not. A literal two-character
/// `\n` counts as whitespace too — asked to escape newlines, a critic that
/// escapes them twice has still copied the line, and the decoded backslash-n
/// is packaging, not a change to the words.
fn collapse_whitespace(text: &str) -> String {
    text.replace("\\n", " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// The blocking finding for a false verdict. Names every claiming task, so
/// the reader sees that the obligation is owned and still not delivered, and
/// quotes the loophole so the fix is the sentence, not a search.
pub fn fidelity_finding(verdict: &FidelityVerdict, claiming_task_ids: &[String]) -> String {
    format!(
        "obligation {} is claimed by {} but none is obliged to make it true — {} — task {}: \"{}\"",
        verdict.obligation_id,
        claiming_task_ids.join(", "),
        verdict.reason.trim(),
        verdict.weakest_task_id,
        verdict.quoted_task_text.trim()
    )
}

/// Identity of one audited cluster: the obligation texts, the claiming task
/// texts, in order, and the skeleton section. Anything that changes the
/// question changes the key, and nothing else does — so re-running lint over
/// an unchanged set is free, and re-freezing the skeleton re-asks.
pub fn fidelity_cluster_digest(
    obligations: &[ClaimedObligation],
    tasks: &[ClaimingTask],
    skeleton: &SkeletonSummary,
) -> String {
    let mut hasher = blake3::Hasher::new();
    for obligation in obligations {
        hasher.update(obligation.id.as_bytes());
        hasher.update(b"\0");
        hasher.update(obligation.text.as_bytes());
        hasher.update(b"\0");
    }
    hasher.update(b"\0tasks\0");
    for task in tasks {
        hasher.update(task.task_id.as_bytes());
        hasher.update(b"\0");
        hasher.update(task.text.as_bytes());
        hasher.update(b"\0");
    }
    hasher.update(b"\0skeleton\0");
    hasher.update(skeleton.as_str().as_bytes());
    hasher.finalize().to_hex().to_string()
}

#[cfg(test)]
#[path = "fidelity_audit_tests.rs"]
mod tests;
