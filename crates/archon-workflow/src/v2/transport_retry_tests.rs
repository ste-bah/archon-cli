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
