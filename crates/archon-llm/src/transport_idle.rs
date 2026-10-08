//! A transport read-idle expiration, never a verdict on the requested work.
#[derive(Debug, thiserror::Error)]
#[error("provider transport stalled: no response bytes before the read-idle backstop; resumable")]
pub struct TransportIdle;

/// Present an initial read-idle expiration through the same stream boundary as
/// a body expiration. No provider-private error text decides classification.
pub(crate) fn receiver() -> tokio::sync::mpsc::Receiver<crate::streaming::StreamEvent> {
    let (tx, rx) = tokio::sync::mpsc::channel(1);
    let _ = tx.try_send(crate::streaming::StreamEvent::Error {
        error_type: "transport_idle".into(),
        message: TransportIdle.to_string(),
    });
    rx
}

/// Issue 364: the text a session ends with when the provider gave no answer
/// for a whole no-progress window, however many resends it took (a network
/// that is not up yet after a wake, a provider that is down). Nothing was
/// wrong with the request, and the work is not judged: callers pause the run
/// (resumable) on it and never re-ask at once. The text crosses process and
/// crate boundaries as a string, so its callers match on this marker.
pub const TRANSPORT_STALL_MARKER: &str =
    "provider transport stall (no answer for a whole no-progress window; resumable):";
