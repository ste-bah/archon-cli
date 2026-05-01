//! Stdio transport for IDE-spawned Archon process (TASK-CLI-411).
//!
//! Reads JSON-RPC requests from a [`BufRead`] line-by-line and writes
//! JSON-RPC responses to a [`Write`] target. Uses JSON-lines framing:
//! each line is one complete JSON-RPC message.
//!
//! Designed to be generic so tests can use [`std::io::Cursor`]/[`Vec<u8>`]
//! instead of `stdin`/`stdout`.

use std::io::{BufRead, Write};

use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

use archon_core::agent::TimestampedEvent;

use crate::ide::handler::{IdeProtocolHandler, event_to_notification};

/// Stdio transport: reads JSON-RPC requests line-by-line, writes responses.
pub struct StdioTransport {
    handler: IdeProtocolHandler,
}

impl StdioTransport {
    /// Create a new transport wrapping `handler`.
    pub fn new(handler: IdeProtocolHandler) -> Self {
        Self { handler }
    }

    /// Run the stdio loop until EOF.
    ///
    /// Reads complete JSON-RPC messages (one per line) from `reader`, passes
    /// each to the [`IdeProtocolHandler`], and writes the response line to
    /// `writer`. Blank lines are silently skipped.
    ///
    /// Returns `Ok(())` on EOF or when the reader is exhausted.
    pub fn run<R: BufRead, W: Write>(&mut self, reader: R, writer: &mut W) -> anyhow::Result<()> {
        for line_result in reader.lines() {
            let line = line_result?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            let response = self.handler.handle(trimmed);
            writer.write_all(response.as_bytes())?;
            writer.write_all(b"\n")?;
            writer.flush()?;
        }
        Ok(())
    }

    /// Run an async stdio loop that handles both incoming requests and outgoing
    /// agent event notifications.
    ///
    /// - `event_rx`: receives `AgentEvent`s from the agent loop
    /// - `session_id`: the active session ID for notification routing
    ///
    /// The loop terminates when stdin reaches EOF or the event channel closes.
    pub async fn run_with_events(
        &mut self,
        mut event_rx: mpsc::UnboundedReceiver<TimestampedEvent>,
        session_id: &str,
    ) -> anyhow::Result<()> {
        let stdin = tokio::io::stdin();
        let mut stdout = tokio::io::stdout();
        let mut reader = tokio::io::BufReader::new(stdin);
        let mut line_buf = String::new();

        loop {
            tokio::select! {
                // Incoming JSON-RPC request from IDE
                result = reader.read_line(&mut line_buf) => {
                    match result {
                        Ok(0) => break, // EOF
                        Ok(_) => {
                            let trimmed = line_buf.trim();
                            if !trimmed.is_empty() {
                                let response = self.handler.handle(trimmed);
                                stdout.write_all(response.as_bytes()).await?;
                                stdout.write_all(b"\n").await?;
                                stdout.flush().await?;
                            }
                            line_buf.clear();
                        }
                        Err(e) => return Err(e.into()),
                    }
                }
                // Outgoing agent event → IDE notification
                event = event_rx.recv() => {
                    match event {
                        Some(evt) => {
                            if let Some(notification) = event_to_notification(session_id, &evt.inner)
                                && let Ok(json) = serde_json::to_string(&notification) {
                                    stdout.write_all(json.as_bytes()).await?;
                                    stdout.write_all(b"\n").await?;
                                    stdout.flush().await?;
                                }
                        }
                        None => break, // Channel closed
                    }
                }
            }
        }

        Ok(())
    }
}
