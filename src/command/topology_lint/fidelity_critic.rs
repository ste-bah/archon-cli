//! The fidelity critic call, split from `fidelity.rs` so the audit file
//! stays under its size budget. Nothing here decides what is asked or what
//! a verdict means: `fidelity.rs` builds the clusters and reads the
//! answers; this file asks and bounds. Verdicts are remembered by
//! `fidelity_store.rs`.

use std::collections::BTreeSet;
use std::path::Path;
use std::time::Duration;

use anyhow::{Result, anyhow};
use archon_workflow::fidelity_audit::{
    ClaimedObligation, ClaimingTask, DOCUMENT_KEYS, FidelityRefusal, FidelityVerdict, RefusalKind,
    SkeletonSummary, VERDICT_KEYS, fidelity_prompt, parse_fidelity_verdicts,
};
use archon_workflow::llm_client_port::{WorkflowAgentOutcome, WorkflowLlmClient};

use crate::command::workflow_freeze_budget::FreezeBudget;

/// The alias the audit asks for. The critic reads whole task files and is
/// asked to find the sentence that lets a claim go hollow; that is the
/// strongest tier the provider offers, whatever it resolves to.
pub(super) const CRITIC_MODEL_ALIAS: &str = "opus";
/// Rejected replies in a row that make no progress before the failure is
/// operational. A reply is read verdict by verdict (#434d): each verdict
/// that passes is kept, and only the obligations still without one are
/// asked again. Each refusal has a class: for a document refused as a whole,
/// the verdict its first error is in (or the document) and its kind; for a
/// verdict refused alone, its obligation id and its kind — kinds from the
/// closed set [`archon_workflow::fidelity_audit::RefusalKind`]. A reply makes
/// progress when it leaves fewer obligations without a verdict, or when one
/// of its refusals has a new class — not among the classes of earlier
/// replies. A new error kind in a verdict is progress even if that verdict
/// comes earlier than the last error (a shorter reply that fixed a later
/// verdict). The same classes again is no progress, whatever their text: a
/// new unknown key name in the same verdict, or a byte-identical reply.
/// There are at most (2 × verdicts + 1) × 46 classes (the document and each
/// reply position, and each obligation, times 46 kinds), and the set without
/// a verdict shrinks at most once per obligation, so progress is bounded by
/// the definition itself; there is no fixed total on attempts, and only this
/// window ends a run of no progress. Observed live: one stray empty key in a
/// complete document, re-sent unchanged at temperature 0.0, came back
/// byte-identical and ended a fixed two-attempt loop.
pub(super) const FIDELITY_NO_PROGRESS_WINDOW: usize = 3;
const CRITIC_TEMPERATURE: f64 = 0.0;
const FIDELITY_CALL_TIMEOUT_SECS: u64 =
    crate::command::workflow_task_set::judge::JUDGE_TIMEOUT_SECS;

/// Shared by cache identity and the actual critic call, including the template.
pub(super) fn request(
    obligations: &[ClaimedObligation],
    tasks: &[ClaimingTask],
    skeleton: &SkeletonSummary,
) -> archon_llm::provider::LlmRequest {
    archon_llm::provider::LlmRequest {
        model: CRITIC_MODEL_ALIAS.into(),
        messages: vec![
            serde_json::json!({"role": "user", "content": fidelity_prompt(obligations, tasks, skeleton)}),
        ],
        extra: serde_json::json!({"temperature": CRITIC_TEMPERATURE}),
        ..Default::default()
    }
}

/// What one batch's [`ask`] came to.
pub(super) enum Asked {
    /// Every obligation has a verdict, each handed to `accept`.
    Answered,
    /// A call stopped making provider progress. Verdicts accepted before it
    /// were handed to `accept`.
    Stopped,
}

/// One refusal of a reply: of the document as a whole, or of the verdict
/// for one obligation, which fails alone.
enum Refused {
    Document(FidelityRefusal),
    Verdict(String, FidelityRefusal),
}

/// What a refusal's class is judged by: see [`FIDELITY_NO_PROGRESS_WINDOW`].
type Class = (Option<String>, Option<usize>, RefusalKind);

impl Refused {
    fn class(&self) -> Class {
        match self {
            Refused::Document(refusal) => (None, refusal.verdict, refusal.kind),
            Refused::Verdict(id, refusal) => (Some(id.clone()), None, refusal.kind),
        }
    }

    fn message(&self) -> &str {
        match self {
            Refused::Document(refusal) | Refused::Verdict(_, refusal) => &refusal.message,
        }
    }
}

/// Ask, and answer a rejected reply: the next request is the original
/// question, the last rejected reply as the critic's turn, and a turn that
/// quotes its exact errors, names the obligations still asked and lists the
/// allowed keys — never the whole history, so its size stays bounded.
///
/// A reply is read verdict by verdict (#434d,
/// [`parse_fidelity_verdicts`]). Every verdict that passes every check is
/// handed to `accept` (with its index in `obligations`) at once, before the
/// next call, and is final: a later reply's verdict for that obligation is
/// never read in its place or merged with it. A document refused as a whole
/// gives no verdict. Ask again, for the obligations still without a verdict,
/// while the replies make progress; after [`FIDELITY_NO_PROGRESS_WINDOW`]
/// replies in a row with no progress the failure is operational and names
/// only those obligations. Every rejected reply is kept under `rejected/`
/// in the cache directory — the operator who is told "no usable verdict"
/// needs to see what the critic actually said.
///
/// Each stream gets the complete no-progress window. A stalled call is
/// `Asked::Stopped`, never a verdict or a candidate defect.
#[allow(clippy::too_many_arguments)]
pub(super) async fn ask(
    client: &dyn WorkflowLlmClient,
    cache: &Path,
    digest: &str,
    obligations: &[ClaimedObligation],
    tasks: &[ClaimingTask],
    skeleton: &SkeletonSummary,
    _budget: &FreezeBudget,
    accept: &mut (dyn FnMut(usize, &FidelityVerdict) -> Result<()> + Send),
) -> Result<Asked> {
    let request = request(obligations, tasks, skeleton);
    let mut messages = request.messages.clone();
    // Whether each obligation has its verdict.
    let mut answered = vec![false; obligations.len()];
    let open = |answered: &[bool]| -> Vec<String> {
        obligations
            .iter()
            .zip(answered)
            .filter(|(_, answered)| !**answered)
            .map(|(obligation, _)| obligation.id.clone())
            .collect()
    };
    let mut seen: BTreeSet<Class> = BTreeSet::new();
    let mut stalled = 0;
    let mut attempt = 0;
    let last = loop {
        attempt += 1;
        let progress = archon_shell::progress::Progress::new(true);
        let outcome = match progress
            .bound(
                Duration::from_secs(FIDELITY_CALL_TIMEOUT_SECS),
                client.send_message_with_progress(
                    messages.clone(),
                    request.system.clone(),
                    request.tools.to_vec(),
                    &request.model,
                    CRITIC_TEMPERATURE,
                    progress.clone(),
                ),
            )
            .await
        {
            Ok(Ok(outcome)) => outcome,
            Ok(Err(archon_workflow::WorkflowError::ControlPaused(_))) => return Ok(Asked::Stopped),
            Ok(Err(error)) => return Err(anyhow::Error::new(error)),
            Err(_) => return Ok(Asked::Stopped),
        };
        // A truncated reply is not re-asked: the budget that cut it off has
        // not changed, and partial JSON is never repaired into a verdict.
        require_complete(&outcome)?;
        let document = crate::command::workflow_freeze_candidate::candidate_document(
            outcome.content.trim().as_bytes(),
        );
        let document = String::from_utf8_lossy(&document);
        let before = open(&answered).len();
        let mut refused = Vec::new();
        match parse_fidelity_verdicts(&document, obligations, tasks) {
            Ok(read) => {
                for (index, result) in read.into_iter().enumerate() {
                    // Answered by an earlier reply: final, never replaced.
                    if answered[index] {
                        continue;
                    }
                    match result {
                        Ok(verdict) => {
                            accept(index, &verdict)?;
                            answered[index] = true;
                        }
                        Err(refusal) => {
                            refused.push(Refused::Verdict(obligations[index].id.clone(), refusal))
                        }
                    }
                }
            }
            Err(refusal) => refused.push(Refused::Document(refusal)),
        }
        if refused.is_empty() {
            return Ok(Asked::Answered);
        }
        let rejected = cache.join("rejected");
        let path = rejected.join(format!("{digest}-attempt-{attempt}.txt"));
        let kept = std::fs::create_dir_all(&rejected)
            .and_then(|()| std::fs::write(&path, &outcome.content))
            .map(|()| path.display().to_string())
            .unwrap_or_else(|error| format!("not kept: {error}"));
        let shrank = open(&answered).len() < before;
        // Every class is recorded, not only up to the first new one.
        let new_class = refused
            .iter()
            .fold(false, |new, refusal| seen.insert(refusal.class()) || new);
        if shrank || new_class {
            stalled = 0;
        } else {
            stalled += 1;
        }
        let errors: Vec<&str> = refused.iter().map(Refused::message).collect();
        if stalled >= FIDELITY_NO_PROGRESS_WINDOW {
            break format!("{} (reply kept at {kept})", errors.join("; "));
        }
        // A provider may refuse an empty assistant turn; say what it was.
        let said = if outcome.content.trim().is_empty() {
            "(an empty reply)".to_string()
        } else {
            outcome.content
        };
        messages = request.messages.clone();
        messages.push(serde_json::json!({"role": "assistant", "content": said}));
        messages.push(
            serde_json::json!({"role": "user", "content": reask(&refused, &open(&answered))}),
        );
    };
    Err(anyhow!(
        "fidelity critic returned no usable verdict for {:?} after {attempt} attempts, the last {FIDELITY_NO_PROGRESS_WINDOW} without progress: {last}",
        open(&answered)
    ))
}

/// The turn that answers a rejected reply: its exact errors, the
/// obligations still asked, and the keys the document and each verdict may
/// have. A verdict already kept is not named again.
fn reask(refused: &[Refused], open: &[String]) -> String {
    let refusal = match refused {
        [Refused::Document(refusal)] => format!(
            "Your reply above was refused, and no verdict was taken from it: {}",
            refusal.message
        ),
        _ => {
            let lines: Vec<String> = refused
                .iter()
                .map(|refusal| match refusal {
                    Refused::Verdict(id, refusal) => format!("- {id}: {}", refusal.message),
                    Refused::Document(refusal) => format!("- {}", refusal.message),
                })
                .collect();
            format!(
                "Your reply above was refused for these obligations, and no verdict was taken for them:\n{}\n\nEvery other verdict in it passed every check and is kept; it is not asked again.",
                lines.join("\n")
            )
        }
    };
    format!(
        "{refusal}\n\nReply again with the whole JSON document and nothing else, holding exactly one verdict for each of these obligation ids and no others: {}; every rule of the first message still applies. The document has exactly one key: {}. Each verdict has exactly these keys and no others: {}.",
        open.join(", "),
        DOCUMENT_KEYS.join(", "),
        VERDICT_KEYS.join(", "),
    )
}

fn require_complete(outcome: &WorkflowAgentOutcome) -> Result<()> {
    match outcome.stop_reason.as_deref() {
        Some("end_turn" | "stop" | "completed") => Ok(()),
        Some(reason) => Err(anyhow!(
            "fidelity critic reply ended with stop reason '{reason}'; a truncated verdict is never parsed"
        )),
        None => Err(anyhow!(
            "fidelity critic returned no finish reason; refusing to parse a possibly truncated verdict"
        )),
    }
}

#[cfg(test)]
#[path = "fidelity_provider_tests.rs"]
mod provider_tests;
#[cfg(test)]
#[path = "fidelity_transport_tests.rs"]
mod transport_tests;
