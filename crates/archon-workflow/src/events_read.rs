use std::path::Path;

use crate::error::WorkflowResult;
use crate::events::WorkflowEvent;

/// Read the parseable records from an append-only event log. A malformed or
/// torn line is counted and skipped so callers see the same history that the
/// workflow history reader exposes.
pub fn read_event_log(path: &Path) -> WorkflowResult<(Vec<WorkflowEvent>, usize)> {
    let raw = std::fs::read(path).map_err(|error| crate::error::WorkflowError::io(path, error))?;
    let mut events = Vec::new();
    let mut damaged_event_lines = 0;
    for (index, line) in raw.split(|byte| *byte == b'\n').enumerate() {
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        match serde_json::from_slice::<WorkflowEvent>(line) {
            Ok(event) => events.push(event),
            Err(error) => {
                damaged_event_lines += 1;
                tracing::warn!(path = %path.display(), line = index + 1, %error,
                    "Skipping malformed workflow event during history lookup");
            }
        }
    }
    Ok((events, damaged_event_lines))
}
