//! The fidelity critic call, split from `fidelity.rs` so the audit file
//! stays under its size budget. Nothing here decides what is asked or what
//! a verdict means: `fidelity.rs` builds the clusters and reads the
//! answers; this file asks and bounds. Verdicts are remembered by
//! `fidelity_store.rs`.

use std::path::Path;
use std::time::Duration;

use anyhow::{Result, anyhow};
use archon_workflow::fidelity_audit::{
    ClaimedObligation, ClaimingTask, FidelityVerdict, SkeletonSummary, fidelity_prompt,
    parse_fidelity_response,
};
use archon_workflow::llm_client_port::{WorkflowAgentOutcome, WorkflowLlmClient};

use crate::command::workflow_freeze_budget::FreezeBudget;

/// The alias the audit asks for. The critic reads whole task files and is
/// asked to find the sentence that lets a claim go hollow; that is the
/// strongest tier the provider offers, whatever it resolves to.
pub(super) const CRITIC_MODEL_ALIAS: &str = "opus";
/// One re-ask on a malformed reply, then the failure is operational. A
/// formatting slip often corrects on a second pass; a third pass is spend.
pub(super) const FIDELITY_ATTEMPTS: usize = 2;
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

/// Ask once, re-ask once on a malformed reply, and keep every rejected reply
/// under `rejected/` in the cache directory — the operator who is told "no
/// usable verdict" needs to see what the critic actually said.
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
    let mut last = String::from("never asked");
    for attempt in 1..=FIDELITY_ATTEMPTS {
        let progress = archon_shell::progress::Progress::new(true);
        let outcome = match progress
            .bound(
                Duration::from_secs(FIDELITY_CALL_TIMEOUT_SECS),
                client.send_message_with_progress(
                    request.messages.clone(),
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
        match parse_fidelity_response(&document, obligations, tasks) {
            Ok(verdicts) => return Ok(Asked::Answered(verdicts)),
            Err(error) => {
                let rejected = cache.join("rejected");
                let path = rejected.join(format!("{digest}-attempt-{attempt}.txt"));
                let kept = std::fs::create_dir_all(&rejected)
                    .and_then(|()| std::fs::write(&path, &outcome.content))
                    .map(|()| path.display().to_string())
                    .unwrap_or_else(|error| format!("not kept: {error}"));
                last = format!("{error} (reply kept at {kept})");
            }
        }
    }
    Err(anyhow!(
        "fidelity critic returned no usable verdict for {:?} after {FIDELITY_ATTEMPTS} attempts: {last}",
        obligations
            .iter()
            .map(|o| o.id.as_str())
            .collect::<Vec<_>>()
    ))
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
#[path = "fidelity_transport_tests.rs"]
mod transport_tests;
