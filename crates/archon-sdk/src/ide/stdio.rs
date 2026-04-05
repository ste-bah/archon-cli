//! Stdio transport for IDE-spawned Archon process (TASK-CLI-411).
//!
//! Reads JSON-RPC requests from a [`BufRead`] line-by-line and writes
//! JSON-RPC responses to a [`Write`] target. Uses JSON-lines framing:
//! each line is one complete JSON-RPC message.
//!
//! Designed to be generic so tests can use [`std::io::Cursor`]/[`Vec<u8>`]
//! instead of `stdin`/`stdout`.

use std::io::{BufRead, Write};

use crate::ide::handler::IdeProtocolHandler;

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
}
