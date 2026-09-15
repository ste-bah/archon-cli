//! Sidecar records: the input head the guard remembers for a call, and the
//! append that writes one record to the per-call read-set file.
use super::{RECORD_HEAD_CHARS, normalise_command};
use serde_json::Value;
use std::io::Write;
use std::path::Path;

/// The part of a tool INPUT worth remembering: the command for Bash, the
/// path or pattern for the inspection tools, the first string field for
/// anything else. Whitespace collapsed and clipped; never tool output.
pub(super) fn record_head(name: &str, input: &Value) -> String {
    let keys: &[&str] = if name == "Bash" {
        &["command"]
    } else {
        &["file_path", "path", "pattern", "query", "command", "url"]
    };
    let text = keys
        .iter()
        .find_map(|key| input.get(*key).and_then(Value::as_str))
        .or_else(|| {
            input
                .as_object()
                .and_then(|object| object.values().find_map(Value::as_str))
        })
        .unwrap_or("");
    clip(&normalise_command(text), RECORD_HEAD_CHARS)
}

pub(super) fn first_line(text: &str) -> String {
    clip(text.lines().next().unwrap_or("").trim(), RECORD_HEAD_CHARS)
}

/// At most `chars` characters, marked when cut.
pub(super) fn clip(text: &str, chars: usize) -> String {
    if text.chars().count() <= chars {
        return text.to_string();
    }
    let mut cut: String = text.chars().take(chars.saturating_sub(1)).collect();
    cut.push('\u{2026}');
    cut
}

pub(super) fn append_record(path: &Path, record: &Value) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    let mut bytes = serde_json::to_vec(record)?;
    bytes.push(b'\n');
    file.write_all(&bytes)
}
