use super::*;

/// Preserve host-owned loop/timeout completion, while provider transport
/// stalls remain resumable errors. Empty reply exhaustion is settled by the
/// runner before it reaches this boundary.
pub(super) fn retained_completion(
    error: &WorkflowV2AgentError,
    records: Option<&archon_workflow::v2::record_landing::RecordLanding>,
) -> Option<archon_workflow::WorkflowAgentOutcome> {
    use archon_workflow::error as call_error;
    let text = error.to_string();
    if text.contains(archon_llm::transport_idle::TRANSPORT_STALL_MARKER) {
        return None;
    }
    let stopped = call_error::is_host_call_timeout_text(&text)
        || call_error::is_no_progress_stop_text(&text)
        || call_error::is_inactivity_timeout_text(&text)
        || call_error::is_read_wall_thrash_text(&text)
        || text.contains("reached max turns");
    let completion = records.filter(|_| stopped)?.retained_reply().ok()??;
    Some(archon_workflow::WorkflowAgentOutcome {
        content: completion.to_string(),
        stop_reason: Some("end_turn".into()),
        ..Default::default()
    })
}
