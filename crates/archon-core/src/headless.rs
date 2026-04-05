use crate::remote::protocol::AgentMessage;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// Headless mode runtime — no TUI, JSON-lines on stdin/stdout.
/// Used as the backend process for remote agent sessions.
pub struct HeadlessRuntime {
    pub session_id: String,
}

impl HeadlessRuntime {
    pub fn new(session_id: String) -> Self {
        Self { session_id }
    }

    /// Run the headless event loop until stdin is closed (EOF).
    pub async fn run(self) -> anyhow::Result<()> {
        tracing::info!("headless: starting session_id={}", self.session_id);

        let stdin = tokio::io::stdin();
        let mut reader = BufReader::new(stdin);
        let mut stdout = tokio::io::stdout();
        let mut line = String::new();

        loop {
            line.clear();
            let n = reader.read_line(&mut line).await?;
            if n == 0 {
                tracing::info!("headless: stdin closed (EOF)");
                break;
            }

            match AgentMessage::from_json_line(&line) {
                Ok(AgentMessage::Ping) => {
                    let pong = AgentMessage::Pong.to_json_line()?;
                    stdout.write_all(pong.as_bytes()).await?;
                    stdout.flush().await?;
                }
                Ok(AgentMessage::UserMessage { content }) => {
                    tracing::info!("headless: user message len={}", content.len());
                    let ack = AgentMessage::Event {
                        kind: "session_start".to_string(),
                        data: serde_json::json!({"session_id": self.session_id}),
                    }
                    .to_json_line()?;
                    stdout.write_all(ack.as_bytes()).await?;
                    stdout.flush().await?;
                    tracing::info!("headless: agent loop deferred to phase 6");
                }
                Ok(_) => {
                    tracing::debug!("headless: ignored non-user message");
                }
                Err(e) => {
                    let err_msg = AgentMessage::Error {
                        message: format!("parse error: {e}"),
                    }
                    .to_json_line()?;
                    stdout.write_all(err_msg.as_bytes()).await?;
                    stdout.flush().await?;
                    tracing::warn!("headless: parse error: {e}");
                }
            }
        }

        Ok(())
    }
}
