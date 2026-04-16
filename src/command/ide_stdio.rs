use archon_core::agent::TimestampedEvent;
use archon_sdk::ide::handler::IdeProtocolHandler;
use archon_sdk::ide::stdio::StdioTransport;

pub(crate) async fn handle_ide_stdio_command() -> anyhow::Result<()> {
    let handler = IdeProtocolHandler::new(env!("CARGO_PKG_VERSION"));
    let mut transport = StdioTransport::new(handler);
    let (_event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel::<TimestampedEvent>();
    let session_id = uuid::Uuid::new_v4().to_string();
    tracing::info!("IDE stdio mode: session={session_id}");
    if let Err(e) = transport.run_with_events(event_rx, &session_id).await {
        tracing::error!("IDE stdio error: {e}");
        return Err(e);
    }
    Ok(())
}
