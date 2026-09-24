//! What the host records about an agent call that did not return a usable
//! result: its own cuts — the wall clock and the inactivity bound — told apart
//! from a provider failure, and the bodies it rejected.

use super::*;

/// Did the host's own per-dispatch timer produce this error?
///
/// One predicate for the two things that must agree: the `call_timeout`
/// transport row and the typed [`WorkflowError::HostCallTimeout`] the port
/// returns. The texts are the pipeline's when `AgentExecutionRequest::
/// timeout_secs` fires (`subagent_adapter::llm_response_for_subagent_outcome`
/// and the runner's turn-boundary check) plus the raw author path's deadline;
/// the typed variant's own marker counts so a cut already typed once stays
/// typed through any wrapper.
///
/// An inactivity cut is the host's too — its own bound, fed by the runner's
/// activity — so it is typed the same way and no transport re-ask restarts it.
/// Its record is its own kind; see [`host_call_timeout_record`].
pub(crate) fn is_host_call_timeout(error: &str) -> bool {
    if archon_workflow::error::is_host_call_timeout_text(error)
        || archon_workflow::error::is_inactivity_timeout_text(error)
    {
        return true;
    }
    let lower = error.to_ascii_lowercase();
    lower.contains("subagent timed out after")
        || lower.contains("wall-clock timeout")
        || lower.contains("deadline exceeded after")
}

/// The `call_timeout` transport row for an error the host's own per-dispatch
/// timer produced, or `None` for every other failure — the same predicate
/// that types the error the port returns for it.
pub(crate) fn host_call_timeout_record(
    call_id: &str,
    error: &str,
    limit_secs: Option<u64>,
    source: &str,
    elapsed_secs: u64,
) -> Option<serde_json::Value> {
    // The inactivity cut first: its text may sit inside a wrapper that also
    // carries the wall-clock marker, and the record must name the bound that
    // actually fired.
    if archon_workflow::error::is_inactivity_timeout_text(error) {
        return Some(serde_json::json!({
            "kind": "call_inactivity_timeout",
            "call_id": call_id,
            "wall_clock_limit_secs": limit_secs,
            "elapsed_secs": elapsed_secs,
            "source": "subagent.inactivity_timeout_secs",
            "error": error,
        }));
    }
    is_host_call_timeout(error).then(|| {
        serde_json::json!({
            "kind": "call_timeout",
            "call_id": call_id,
            "limit_secs": limit_secs,
            "elapsed_secs": elapsed_secs,
            "source": source,
        })
    })
}

/// Persist an agent body that was rejected, for ANY branch role.
///
/// This used to return early unless the request was write-capable, so a
/// verification branch destroyed by schema repair left nothing behind: the run
/// directory recorded rejected outputs for every `implement-*` branch and none
/// for any `verification-wave-*`. When a live verification died on a single
/// unrecognised enum value, the body that would have named it in one line was
/// already gone, and the cause had to be reconstructed from the error string.
///
/// A read-only branch's body is worth exactly as much as a write branch's here:
/// the artefact being diagnosed is the agent's OUTPUT, and whether the agent was
/// allowed to change files says nothing about how useful its output is to read.
/// The disk cost is bounded by the same repair cap either way.
pub(crate) fn save_rejected_output(
    v2_store: Option<&WorkflowV2ResultStore>,
    request: &archon_workflow::WorkflowV2AgentRequest,
    attempt: &str,
    body: &str,
    error: &WorkflowV2AgentError,
) {
    let Some(store) = v2_store else {
        return;
    };
    let record = WorkflowV2RejectedOutput {
        attempt: attempt.to_string(),
        error: error.to_string(),
        raw_body: body.to_string(),
    };
    let _ = store.append_rejected_output(&request.call.id, record);
}

pub(crate) fn save_rejected_write_result(
    v2_store: Option<&WorkflowV2ResultStore>,
    request: &archon_workflow::WorkflowV2AgentRequest,
    attempt: &str,
    body: &str,
    result: &WorkflowV2Result,
) {
    if !result_has_rejected_write_output(result) {
        return;
    }
    save_rejected_output(
        v2_store,
        request,
        attempt,
        body,
        &WorkflowV2AgentError::InvalidResult(result.summary.clone()),
    );
}

fn result_has_rejected_write_output(result: &WorkflowV2Result) -> bool {
    result.residual_gaps.iter().any(|gap| {
        gap.id.starts_with("invalid_write_branch_output_")
            || gap.description.contains("patch is empty")
            || gap.description.contains("output not usable")
            || gap.description.contains("verification blocked after patch")
            || gap.description.contains("exceeds max")
    })
}
