//! The read side, and its tolerance for a file still being written.
//!
//! Owns [`TraceReadout`] and [`read_trace`]. Everything here exists because a
//! fold may run while workers are still appending: a trailing fragment is
//! discarded and flagged rather than treated as corruption, and a line that
//! will not parse is counted rather than raised.

use std::fs;
use std::io;
use std::path::Path;

use super::record::TraceRecord;

/// What [`read_trace`] recovered.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TraceReadout {
    pub records: Vec<TraceRecord>,
    /// Complete lines that failed to parse. Counted, not fatal.
    pub malformed_lines: usize,
    /// True when the file did not end in a newline, i.e. the last line was
    /// still being written or the process died mid-append. The fragment is
    /// discarded.
    pub truncated_tail: bool,
}

impl TraceReadout {
    /// True when nothing at all was recovered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
}

/// Read a trace, tolerating truncation and unknown record shapes.
///
/// A missing file yields an empty readout rather than an error: a graph that
/// declared itself and then did nothing is legitimate.
///
/// **Only complete lines are parsed.** A trailing fragment with no newline is
/// discarded and flagged, which is what makes a fold safe to run while workers
/// are still appending — the fragment will be complete on the next read, and
/// the fold is idempotent, so nothing is lost by skipping it now.
pub fn read_trace(path: &Path) -> io::Result<TraceReadout> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(TraceReadout::default());
        }
        // A trace containing invalid UTF-8 is a corrupt trace, not a fatal
        // condition; recover what is decodable.
        Err(error) if error.kind() == io::ErrorKind::InvalidData => {
            let bytes = fs::read(path)?;
            String::from_utf8_lossy(&bytes).into_owned()
        }
        Err(error) => return Err(error),
    };

    let mut readout = TraceReadout {
        truncated_tail: !contents.is_empty() && !contents.ends_with('\n'),
        ..TraceReadout::default()
    };

    let complete = match contents.rfind('\n') {
        Some(index) => &contents[..=index],
        // No newline anywhere: every byte present is a fragment.
        None => "",
    };

    for line in complete.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<TraceRecord>(line) {
            Ok(record) => readout.records.push(record),
            Err(_) => readout.malformed_lines += 1,
        }
    }

    Ok(readout)
}
