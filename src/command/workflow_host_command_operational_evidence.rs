use super::OperationalAttempt;

impl OperationalAttempt {
    pub(crate) fn new(
        attempt: u32,
        reason: &'static str,
        elapsed_secs: u64,
        progress: Option<u64>,
    ) -> Self {
        Self {
            attempt,
            reason,
            elapsed_secs,
            progress,
            exit_code: None,
            stderr_tail: String::new(),
        }
    }

    pub(crate) fn with_child_evidence(
        mut self,
        exit_code: Option<i32>,
        stderr_tail: String,
    ) -> Self {
        self.exit_code = exit_code;
        self.stderr_tail = stderr_tail;
        self
    }
}

/// Keep the final child diagnostics, where errors and remediation hints appear.
pub(crate) fn stderr_tail(stderr: &[u8]) -> String {
    const MAX_BYTES: usize = 4096;
    let start = stderr.len().saturating_sub(MAX_BYTES);
    String::from_utf8_lossy(&stderr[start..]).into_owned()
}
