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
//! no domain: it reads only the PRD's obligation text and the task files' own
//! text, and every rule below is about shape and provenance, never content.

use serde::{Deserialize, Serialize};

/// Longest `reason` accepted — a verdict is a pointer to a loophole, not an
/// essay, and a finding that quotes it must stay readable in a log line.
pub const MAX_REASON_CHARS: usize = 400;
/// Longest `quoted_task_text` accepted, verbatim from the weakest task.
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
/// the same claiming tasks so each task's text travels once per cluster.
pub fn fidelity_prompt(obligations: &[ClaimedObligation], tasks: &[ClaimingTask]) -> String {
    let mut prompt = format!(
        "You are auditing whether a task decomposition is faithful to its PRD. Below are PRD obligations, each with the exact text the PRD states, followed by the FULL text of every task file that claims to implement them. Assume every listed task passes its own acceptance criteria and focused tests exactly as written, including every allowance the task text grants itself (temporary roots, pending/deferred statuses, fail-closed residual gaps, minimum counts of zero, optional lanes). For each obligation: is the PRD obligation then necessarily true? Answer strictly: it is necessarily true only when no reading of the task texts lets every task pass while the obligation stays false in the project the PRD describes; a task that satisfies itself somewhere other than where the PRD requires the result, or that permits the result to be absent, deferred or empty, does not make the obligation true. Return JSON only as {{\"verdicts\":[{{\"obligation_id\":\"...\",\"necessarily_true\":true,\"weakest_task_id\":\"...\",\"reason\":\"...\",\"quoted_task_text\":\"...\"}}]}} with exactly one verdict for every obligation id and no extra ids or fields. reason: one line of at most {MAX_REASON_CHARS} characters saying why, never empty — for a true verdict it names what in the task text obliges the result. When necessarily_true is false, weakest_task_id names the listed task whose allowance grants the loophole and quoted_task_text is an excerpt of at most {MAX_QUOTE_CHARS} characters copied verbatim from that task's text, exactly as it appears; when necessarily_true is true, both are empty strings. Every string is a single line with newlines escaped as \\n; emit the JSON document alone.\n\nObligations: {}\n",
        serde_json::to_string(obligations).unwrap_or_default()
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
/// Every defect is an error, never a default: a missing verdict is not a pass,
/// an unknown obligation id is not ignored, a quote that does not appear in
/// the named task is not a quote. The caller re-asks once and then treats the
/// failure as operational — a verdict the host cannot check is a verdict the
/// host does not have.
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
        let verdict = by_id.remove(&obligation.id).expect("id set checked");
        check_verdict(&verdict, tasks)?;
        verdicts.push(verdict);
    }
    Ok(verdicts)
}

fn check_verdict(verdict: &FidelityVerdict, tasks: &[ClaimingTask]) -> Result<(), String> {
    let id = &verdict.obligation_id;
    if verdict.reason.trim().is_empty() {
        return Err(format!("verdict for {id} has an empty reason"));
    }
    if verdict.reason.chars().count() > MAX_REASON_CHARS {
        return Err(format!(
            "verdict for {id} has a reason longer than {MAX_REASON_CHARS} characters"
        ));
    }
    if verdict.quoted_task_text.chars().count() > MAX_QUOTE_CHARS {
        return Err(format!(
            "verdict for {id} quotes more than {MAX_QUOTE_CHARS} characters"
        ));
    }
    let weakest = tasks
        .iter()
        .find(|task| task.task_id == verdict.weakest_task_id);
    if verdict.necessarily_true {
        if !verdict.weakest_task_id.is_empty() && weakest.is_none() {
            return Err(format!(
                "verdict for {id} names weakest task '{}' which does not claim it",
                verdict.weakest_task_id
            ));
        }
        return Ok(());
    }
    let Some(weakest) = weakest else {
        return Err(format!(
            "false verdict for {id} names weakest task '{}' which does not claim it",
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

/// Identity of one audited cluster: the obligation texts and the claiming
/// task texts, in order. Anything that changes the question changes the key,
/// and nothing else does — so re-running lint over an unchanged set is free.
pub fn fidelity_cluster_digest(
    obligations: &[ClaimedObligation],
    tasks: &[ClaimingTask],
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
    hasher.finalize().to_hex().to_string()
}

#[cfg(test)]
#[path = "fidelity_audit_tests.rs"]
mod tests;
