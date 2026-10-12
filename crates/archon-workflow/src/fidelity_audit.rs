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

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::task_skeleton::TaskSkeleton;

/// Longest `reason` kept — a verdict is a pointer to a loophole, not an
/// essay, and a finding that quotes it must stay readable in a log line. A
/// longer reason is cut here, never refused (Issue-42).
pub const MAX_REASON_CHARS: usize = 400;
/// Longest `quoted_task_text` kept, verbatim from the weakest task.
pub const MAX_QUOTE_CHARS: usize = 300;
/// Least a true verdict's `supporting_text` may be: this many words, or
/// [`MIN_SUPPORT_CHARS`] characters, so it is a passage a reader can check,
/// not a word that happens to occur (#434).
pub const MIN_SUPPORT_WORDS: usize = 8;
/// See [`MIN_SUPPORT_WORDS`].
pub const MIN_SUPPORT_CHARS: usize = 40;
/// The keys of one verdict, exactly: the fields of [`FidelityVerdict`],
/// which refuses any other. The prompt and the re-ask name them.
pub const VERDICT_KEYS: [&str; 7] = [
    "obligation_id",
    "necessarily_true",
    "weakest_task_id",
    "reason",
    "quoted_task_text",
    "supporting_task_id",
    "supporting_text",
];
/// The keys of the verdict document, exactly.
pub const DOCUMENT_KEYS: [&str; 1] = ["verdicts"];

/// One PRD obligation with the exact text the PRD states for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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

/// The typed answer for one obligation, as checked and as stored: every
/// field is present. A critic's reply is read as [`ReplyVerdict`] first, and
/// [`checked_verdict`] decides what an omitted key means.
///
/// # Why a true verdict quotes its evidence (#434)
///
/// A false verdict always quoted the loophole; a true verdict quoted
/// nothing, so a reader could not see what the critic held to oblige the
/// result. A true verdict now names the task whose text obliges it
/// (`supporting_task_id`) and copies those words (`supporting_text`); the
/// host checks the quote is verbatim, exactly as it checks a loophole quote,
/// and that it is an anchor a reader can check (long enough, once in the
/// task, in its prose). The quote is evidence only: a stored verdict is
/// reused solely over byte-identical cluster texts, because a qualifier
/// beside the quote or a loophole elsewhere can make the obligation false
/// while the quote stays.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FidelityVerdict {
    pub obligation_id: String,
    pub necessarily_true: bool,
    pub weakest_task_id: String,
    pub reason: String,
    pub quoted_task_text: String,
    pub supporting_task_id: String,
    pub supporting_text: String,
}

#[path = "fidelity_audit_reply.rs"]
mod reply;
pub use reply::{FidelityRefusal, RefusalKind};
use reply::{ReplyVerdict, checked_verdict, json_kind, shape_class};

#[path = "fidelity_audit_check.rs"]
mod check;
use check::check_verdict;
pub use check::verdict_holds;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FidelityResponse {
    verdicts: Vec<ReplyVerdict>,
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
        "You are auditing whether a task decomposition is faithful to its PRD. Below are PRD obligations, each with the exact text the PRD states, followed by the FROZEN SKELETON of the whole task set, followed by the FULL text of every task file that claims to implement them or that a claiming task names as owning part of the result. Assume every listed task passes its own acceptance criteria and focused tests exactly as written, including every allowance the task text grants itself (temporary roots, pending/deferred statuses, fail-closed residual gaps, minimum counts of zero, optional lanes). For each obligation: is the PRD obligation then necessarily true? Answer strictly: it is necessarily true only when no reading of the task texts lets every task pass while the obligation stays false in the project the PRD describes; a task that satisfies itself somewhere other than where the PRD requires the result, or that permits the result to be absent, deferred or empty, does not make the obligation true. Return JSON only as {{\"verdicts\":[{{\"obligation_id\":\"...\",\"necessarily_true\":true,\"weakest_task_id\":\"...\",\"reason\":\"...\",\"quoted_task_text\":\"...\",\"supporting_task_id\":\"...\",\"supporting_text\":\"...\"}}]}} with exactly one verdict for every obligation id and no extra ids or fields. Each verdict has exactly these keys and no others: {verdict_keys}. reason: one line of at most {MAX_REASON_CHARS} characters saying why, never empty — for a true verdict it names what in the task text obliges the result. When necessarily_true is false, weakest_task_id names the listed task whose allowance grants the loophole and quoted_task_text is an excerpt of at most {MAX_QUOTE_CHARS} characters copied verbatim from that task's text, exactly as it appears; supporting_task_id and supporting_text are empty strings. When necessarily_true is true, supporting_task_id names the listed task whose text obliges the result and supporting_text is an excerpt of at most {MAX_QUOTE_CHARS} characters copied verbatim from that task's text, exactly as it appears: at least {MIN_SUPPORT_WORDS} words of the task's prose that oblige the result and occur only once in that task, never a heading, a code block or an HTML comment; weakest_task_id and quoted_task_text are empty strings. Every string is a single line with newlines escaped as \\n; emit the JSON document alone.\n\nThe FROZEN SKELETON lists every task in the set with its frozen depends_on, blocks, implements and deliverable_contracts. Inter-task ordering and result ownership are FACTS established by the frozen skeleton, not allowances in task prose: depends_on is transitive, so a task runs after everything its dependencies depend on, and a task's implements and deliverable_contracts say what it owns. A task listed in the skeleton whose full text is not included below is not yet written; it will be audited when it is written and again at the set gate, so its absence is never by itself a ground to refute an obligation. Judge the allowances in the task texts that ARE included, against the ordering and ownership the skeleton establishes, and never refute an obligation on ordering or ownership grounds the skeleton already guarantees. When the set has no frozen skeleton the section says so, and only the task texts establish ordering and ownership.\n\nREPOSITORY PATH CLAIMS: the task set was decomposed against one code repository at one recorded base commit, and a task text that says a backticked repository path exists, or does not exist and will be created, is making a claim the host checks deterministically against that repository and refuses when it is wrong. You are not shown the repository. Never refute or accept an obligation on your own belief about whether a named file exists; take a task's existence claims as the host-checked facts they are and judge only whether, given them, the task text obliges the result where the PRD requires it.\n\nObligations: {}\n\n{}",
        serde_json::to_string(obligations).unwrap_or_default(),
        skeleton.as_str(),
        verdict_keys = VERDICT_KEYS.join(", "),
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
/// not appear in the named task is not a quote. A shape error names its path
/// (`verdicts[1].weakest_task_text`), because the caller shows the critic
/// this error and asks again while its replies make progress, then treats the
/// failure as operational — a verdict the host cannot check is a verdict the
/// host does not have. Length alone is not a defect: an
/// over-long reason or quote is cut to its limit and kept (see
/// [`check_verdict`]).
pub fn parse_fidelity_response(
    document: &str,
    obligations: &[ClaimedObligation],
    tasks: &[ClaimingTask],
) -> Result<Vec<FidelityVerdict>, String> {
    parse_fidelity_reply(document, obligations, tasks).map_err(|refusal| refusal.message)
}

/// [`parse_fidelity_response`], with where a refused reply stopped parsing.
pub fn parse_fidelity_reply(
    document: &str,
    obligations: &[ClaimedObligation],
    tasks: &[ClaimingTask],
) -> Result<Vec<FidelityVerdict>, FidelityRefusal> {
    let mut by_id = read_document(document, obligations)?;
    let expected: BTreeSet<&str> = obligations.iter().map(|o| o.id.as_str()).collect();
    let actual: BTreeSet<&str> = by_id.keys().map(String::as_str).collect();
    if expected != actual {
        return Err(id_mismatch(&expected, &actual));
    }
    // The ids match the cluster with none repeated, so every reply index is
    // below the cluster size.
    obligations
        .iter()
        .map(|obligation| {
            let (index, reply) = by_id.remove(&obligation.id).expect("id set checked");
            checked(reply, index, tasks)
        })
        .collect()
}

/// A reply read verdict by verdict (#434d): one result per obligation, in
/// `obligations` order, or the refusal of the document as a whole.
///
/// The document is refused as a whole, and no verdict is taken from it, for
/// everything [`parse_fidelity_reply`] refuses before it reads a verdict's
/// meaning: a reply that is not JSON or not exactly the verdict document,
/// any key, type or shape error in any verdict (serde reads the whole
/// document at once), an obligation id that repeats, and an id outside
/// `obligations`. What is left is one verdict per obligation id, read
/// strictly; each then passes or fails on its own, by exactly the checks
/// [`parse_fidelity_reply`] applies to it (what an omitted key means, the
/// named task is in the cluster, its quote is verbatim, a true verdict's
/// anchor). An obligation the reply has no verdict for fails alone, as
/// [`RefusalKind::WrongObligationIds`]. So a verdict this accepts is one
/// [`parse_fidelity_reply`] would accept in a reply whose other verdicts
/// all passed.
pub fn parse_fidelity_verdicts(
    document: &str,
    obligations: &[ClaimedObligation],
    tasks: &[ClaimingTask],
) -> Result<Vec<Result<FidelityVerdict, FidelityRefusal>>, FidelityRefusal> {
    let mut by_id = read_document(document, obligations)?;
    let expected: BTreeSet<&str> = obligations.iter().map(|o| o.id.as_str()).collect();
    let actual: BTreeSet<&str> = by_id.keys().map(String::as_str).collect();
    if !actual.is_subset(&expected) {
        return Err(id_mismatch(&expected, &actual));
    }
    Ok(obligations
        .iter()
        .map(|obligation| match by_id.remove(&obligation.id) {
            Some((index, reply)) => checked(reply, index, tasks),
            None => Err(FidelityRefusal {
                message: format!("fidelity reply has no verdict for {}", obligation.id),
                verdict: None,
                kind: RefusalKind::WrongObligationIds,
            }),
        })
        .collect())
}

/// The verdict document read strictly, by obligation id with its index in
/// the reply; refused when it does not deserialize or repeats an id.
fn read_document(
    document: &str,
    obligations: &[ClaimedObligation],
) -> Result<BTreeMap<String, (usize, ReplyVerdict)>, FidelityRefusal> {
    let document = document.trim();
    let not_the_document =
        |error: &dyn std::fmt::Display, class: (Option<usize>, RefusalKind)| FidelityRefusal {
            message: format!("fidelity reply is not the verdict document: {error}"),
            verdict: class.0,
            kind: class.1,
        };
    let mut deserializer = serde_json::Deserializer::from_str(document);
    let response: FidelityResponse = serde_path_to_error::deserialize(&mut deserializer)
        .map_err(|error| not_the_document(&error, shape_class(&error, obligations.len())))?;
    deserializer
        .end()
        .map_err(|error| not_the_document(&error, (None, json_kind(&error, None))))?;
    let mut by_id = BTreeMap::new();
    for (index, verdict) in response.verdicts.into_iter().enumerate() {
        if by_id
            .insert(verdict.obligation_id.clone(), (index, verdict))
            .is_some()
        {
            return Err(FidelityRefusal {
                message: "fidelity reply repeats an obligation id".to_string(),
                verdict: None,
                kind: RefusalKind::WrongObligationIds,
            });
        }
    }
    Ok(by_id)
}

fn id_mismatch(expected: &BTreeSet<&str>, actual: &BTreeSet<&str>) -> FidelityRefusal {
    FidelityRefusal {
        message: format!(
            "fidelity reply verdict ids do not match the cluster: missing={:?}, extra={:?}",
            expected.difference(actual).collect::<Vec<_>>(),
            actual.difference(expected).collect::<Vec<_>>()
        ),
        verdict: None,
        kind: RefusalKind::WrongObligationIds,
    }
}

/// One verdict, the reply's `index`-th, through every check of its meaning
/// and provenance.
fn checked(
    reply: ReplyVerdict,
    index: usize,
    tasks: &[ClaimingTask],
) -> Result<FidelityVerdict, FidelityRefusal> {
    checked_verdict(reply)
        .and_then(|mut verdict| {
            check_verdict(&mut verdict, tasks)?;
            Ok(verdict)
        })
        .map_err(|(kind, message)| FidelityRefusal {
            message,
            verdict: Some(index),
            kind,
        })
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

/// The blocking finding for an obligation whose stored verdicts disagree
/// while both still hold on the current texts (#434): a true and a false
/// verdict over the same cluster with byte-identical texts. Neither is taken over the other; the finding quotes both,
/// so the record of the disagreement is the finding itself.
pub fn fidelity_contradiction_finding(
    refuted: &FidelityVerdict,
    supported: &FidelityVerdict,
    claiming_task_ids: &[String],
) -> String {
    format!(
        "obligation {} is claimed by {} and the critic's verdicts contradict each other on unchanged evidence; neither is taken — false: {} — task {}: \"{}\" — true: {} — task {}: \"{}\"",
        refuted.obligation_id,
        claiming_task_ids.join(", "),
        refuted.reason.trim(),
        refuted.weakest_task_id,
        refuted.quoted_task_text.trim(),
        supported.reason.trim(),
        supported.supporting_task_id,
        supported.supporting_text.trim()
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
