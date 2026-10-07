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
