use std::fmt;

/// A single recorded permission denial.
#[derive(Debug, Clone)]
pub struct DenialEntry {
    pub tool_name: String,
    pub reason: String,
    pub timestamp: std::time::SystemTime,
}

/// Append-only log of permission denials for audit / display.
#[derive(Debug, Clone)]
pub struct DenialLog {
    entries: Vec<DenialEntry>,
}

impl DenialLog {
    /// Create a new empty denial log.
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Record a denial event.
    pub fn record(&mut self, tool_name: &str, reason: &str) {
        self.entries.push(DenialEntry {
            tool_name: tool_name.to_owned(),
            reason: reason.to_owned(),
            timestamp: std::time::SystemTime::now(),
        });
    }

    /// Return the most recent `limit` entries (or fewer if the log is shorter).
    pub fn recent(&self, limit: usize) -> &[DenialEntry] {
        let start = self.entries.len().saturating_sub(limit);
        &self.entries[start..]
    }

    /// Format recent denials for human-readable display.
    pub fn format_display(&self, limit: usize) -> String {
        let entries = self.recent(limit);
        if entries.is_empty() {
            return "No recent permission denials.".to_owned();
        }
        let mut out = String::new();
        for (i, entry) in entries.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            fmt::write(
                &mut out,
                format_args!("[{}] {} — {}", i + 1, entry.tool_name, entry.reason),
            )
            .ok();
        }
        out
    }
}

impl Default for DenialLog {
    fn default() -> Self {
        Self::new()
    }
}
