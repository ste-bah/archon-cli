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
    ClaimedObligation, ClaimingTask, DOCUMENT_KEYS, FidelityVerdict, SkeletonSummary, VERDICT_KEYS,
    fidelity_prompt, parse_fidelity_reply,
};
use archon_workflow::llm_client_port::{WorkflowAgentOutcome, WorkflowLlmClient};

use crate::command::workflow_freeze_budget::FreezeBudget;

/// The alias the audit asks for. The critic reads whole task files and is
/// asked to find the sentence that lets a claim go hollow; that is the
/// strongest tier the provider offers, whatever it resolves to.
pub(super) const CRITIC_MODEL_ALIAS: &str = "opus";
/// Rejected replies in a row that make no progress before the failure is
/// operational. Each refused reply's first error has a class: the verdict
/// it is in (or the document as a whole) and its kind, from the closed set
/// [`archon_workflow::fidelity_audit::RefusalKind`]. A reply makes progress
/// when it parses fully or when its class is new — not among the classes of
/// earlier replies. A new error kind in a verdict is progress even if that
/// verdict comes earlier than the last error (a shorter reply that fixed a
/// later verdict). The same class again is no progress, whatever its text:
/// a new unknown key name in the same verdict, or a byte-identical reply.
/// There are at most (verdicts + 1) × 29 classes (each verdict of the
/// cluster plus the document, times 29 kinds), so progress is bounded by the
/// definition itself; there is no fixed total on attempts, and only this
/// window ends a run of no progress. Observed live: one stray empty key in
/// a complete document, re-sent unchanged at temperature 0.0, came back
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
    Answered(Vec<FidelityVerdict>),
    /// A call stopped making provider progress.
    Stopped,
}

/// Ask, and answer a rejected reply: the next request is the original
/// question, the last rejected reply as the critic's turn, and a turn that
/// quotes its exact parse error and lists the allowed keys — never the whole
/// history, so its size stays bounded. Ask again while the replies make
/// progress; after [`FIDELITY_NO_PROGRESS_WINDOW`] replies in a row with no
/// progress the failure is operational. Every rejected reply is kept under
/// `rejected/` in the cache directory — the operator who is told "no usable
/// verdict" needs to see what the critic actually said.
///
/// Each stream gets the complete no-progress window. A stalled call is
/// `Asked::Stopped`, never a verdict or a candidate defect.
pub(super) async fn ask(
    client: &dyn WorkflowLlmClient,
    cache: &Path,
    digest: &str,
    obligations: &[ClaimedObligation],
    tasks: &[ClaimingTask],
    skeleton: &SkeletonSummary,
    _budget: &FreezeBudget,
) -> Result<Asked> {
    let request = request(obligations, tasks, skeleton);
    let mut messages = request.messages.clone();
    let mut seen = BTreeSet::new();
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
        match parse_fidelity_reply(&document, obligations, tasks) {
            Ok(verdicts) => return Ok(Asked::Answered(verdicts)),
            Err(error) => {
                let rejected = cache.join("rejected");
                let path = rejected.join(format!("{digest}-attempt-{attempt}.txt"));
                let kept = std::fs::create_dir_all(&rejected)
                    .and_then(|()| std::fs::write(&path, &outcome.content))
                    .map(|()| path.display().to_string())
                    .unwrap_or_else(|error| format!("not kept: {error}"));
                if seen.insert(error.class()) {
                    stalled = 0;
                } else {
                    stalled += 1;
                }
                if stalled >= FIDELITY_NO_PROGRESS_WINDOW {
                    break format!("{error} (reply kept at {kept})");
                }
                // A provider may refuse an empty assistant turn; say what it was.
                let said = if outcome.content.trim().is_empty() {
                    "(an empty reply)".to_string()
                } else {
                    outcome.content
                };
                messages = request.messages.clone();
                messages.push(serde_json::json!({"role": "assistant", "content": said}));
                messages
                    .push(serde_json::json!({"role": "user", "content": reask(&error.message)}));
            }
        }
    };
    Err(anyhow!(
        "fidelity critic returned no usable verdict for {:?} after {attempt} attempts, the last {FIDELITY_NO_PROGRESS_WINDOW} without progress: {last}",
        obligations
            .iter()
            .map(|o| o.id.as_str())
            .collect::<Vec<_>>()
    ))
}

/// The turn that answers a rejected reply: its exact error and the keys the
/// document and each verdict may have.
fn reask(error: &str) -> String {
    format!(
        "Your reply above was refused, and no verdict was taken from it: {error}\n\nReply again with the whole JSON document and nothing else; every rule of the first message still applies. The document has exactly one key: {}. Each verdict has exactly these keys and no others: {}.",
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
