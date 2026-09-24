use super::*;

const LIVE: &str = "workflow stage failed: agent transport failed: workflow stage failed: subagent failed: Subagent failed: HTTP error: response_failed: Codex response failed";

/// The live failure, twice in one morning, both ending the run.
#[test]
fn the_live_codex_drop_is_transport() {
    assert!(is_transport_failure(LIVE));
    assert!(!is_content_rejection(LIVE));
}

#[test]
fn other_transport_shapes_are_recognised() {
    for e in [
        "connection reset by peer",
        "connection closed before response",
        "stream ended unexpectedly",
    ] {
        assert!(is_transport_failure(e), "{e}");
    }
}

/// A verdict about the work is never retried as transport, however it is
/// wrapped — otherwise a real rejection would be re-asked until the cap.
#[test]
fn a_rejection_about_the_work_is_not_transport() {
    for e in [
        "workflow stage failed: your patch would make source file 'x.rs' 512 lines; the ENTIRE patch is rejected",
        "implementation agent changed files outside declared target_files",
        "agent result failed validation",
    ] {
        assert!(!is_transport_failure(e), "{e}");
        assert!(is_content_rejection(e), "{e}");
    }
}

/// A prompt the provider refuses for its size is refused identically every
/// time; re-asking multiplies the compaction path's recovery requests.
#[test]
fn a_context_window_rejection_is_never_retried_as_transport() {
    for e in [
        "agent transport failed: context window exceeded: maximum context length exceeded",
        "agent transport failed: prompt is too long for this model",
        "agent transport failed: request too large",
    ] {
        assert!(is_content_rejection(e), "{e}");
    }
}

/// The live retry cut: the host's timer ended the session, and the pipeline
/// phrased it as a transport failure. Typed by the host, it is not one.
#[test]
fn a_host_call_timeout_is_never_transport() {
    let cut = crate::WorkflowError::HostCallTimeout(
        "agent transport failed: subagent timed out after 1800s".to_string(),
    )
    .to_string();
    assert!(!is_transport_failure(&cut), "{cut}");
    // Wrapped by a host or a retry layer, the marker still governs.
    let wrapped = format!("workflow stage failed: {cut}");
    assert!(!is_transport_failure(&wrapped), "{wrapped}");
    // The untyped text a host that does not classify would send is still
    // transport — the fix is the type, not a new phrase.
    assert!(is_transport_failure(
        "workflow stage failed: agent transport failed: subagent timed out after 1800s"
    ));
}

/// Issue-54: the tool guard ended the session for thrashing past the read
/// wall. The pipeline wraps that in its transport phrase too; re-asking the
/// same prompt would only restart the thrash.
#[test]
fn a_read_wall_thrash_cut_is_never_transport() {
    let cut = "workflow stage failed: agent transport failed: subagent failed: read-wall thrash: 16 non-writing calls after the read budget was exhausted; 0 substantive writes";
    assert!(!is_transport_failure(cut), "{cut}");
    assert!(!is_content_rejection(cut), "{cut}");
    assert!(crate::error::is_read_wall_thrash_text(cut));
}

/// An inactivity cut arrives inside the pipeline's transport wrapper even on a
/// path that never typed it. It is the host's own cut, re-asked at most once
/// by its caller — never the six re-asks a dropped connection gets.
#[test]
fn an_inactivity_cut_is_never_retried_as_transport() {
    let untyped = format!(
        "agent transport failed: {} no model output, tool call or tool result for 1800s",
        crate::error::INACTIVITY_TIMEOUT_MARKER
    );
    assert!(!is_transport_failure(&untyped), "{untyped}");
    assert!(crate::error::is_inactivity_timeout_text(&untyped));
    assert!(!crate::error::is_host_call_timeout_text(&untyped));
}
