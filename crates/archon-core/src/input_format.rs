use std::io::{BufRead, Read};

// ---------------------------------------------------------------------------
// InputFormat enum
// ---------------------------------------------------------------------------

/// Input format for print mode.
#[derive(Debug, Clone, PartialEq)]
pub enum InputFormat {
    /// Plain text: read all of stdin as a single string.
    Text,
    /// NDJSON: each line is `{"role":"user","content":"..."}`.
    StreamJson,
}

impl InputFormat {
    /// Parse an input format string.
    ///
    /// Accepted values: `"text"`, `"stream-json"`.
    pub fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "text" => Ok(Self::Text),
            "stream-json" => Ok(Self::StreamJson),
            other => Err(format!(
                "unknown input format '{other}': expected text or stream-json"
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// Input reading
// ---------------------------------------------------------------------------

/// Read input from stdin based on the format.
///
/// - `Text`: reads all stdin as a single string (one message).
/// - `StreamJson`: parses NDJSON lines as `{"role":"user","content":"..."}`,
///   returning only user messages.
pub fn read_input(format: &InputFormat) -> Result<Vec<String>, String> {
    let stdin = std::io::stdin();
    let reader = stdin.lock();
    read_input_from_reader(format, reader)
}

/// Read input from an arbitrary reader (testable without real stdin).
pub fn read_input_from_reader<R: Read>(
    format: &InputFormat,
    reader: R,
) -> Result<Vec<String>, String> {
    match format {
        InputFormat::Text => {
            let mut buf = String::new();
            let mut reader = reader;
            reader
                .read_to_string(&mut buf)
                .map_err(|e| format!("failed to read stdin: {e}"))?;
            let trimmed = buf.trim().to_string();
            if trimmed.is_empty() {
                return Err("empty input".to_string());
            }
            Ok(vec![trimmed])
        }
        InputFormat::StreamJson => {
            let buf_reader = std::io::BufReader::new(reader);
            let mut messages = Vec::new();
            for line_result in buf_reader.lines() {
                let line = line_result.map_err(|e| format!("failed to read line: {e}"))?;
                let trimmed = line.trim().to_string();
                if trimmed.is_empty() {
                    continue;
                }
                let parsed: serde_json::Value = serde_json::from_str(&trimmed)
                    .map_err(|e| format!("invalid JSON on input line: {e}"))?;
                let role = parsed.get("role").and_then(|v| v.as_str()).unwrap_or("");
                if role != "user" {
                    continue;
                }
                let content = parsed
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if !content.is_empty() {
                    messages.push(content);
                }
            }
            if messages.is_empty() {
                return Err("no user messages found in stream-json input".to_string());
            }
            Ok(messages)
        }
    }
}
