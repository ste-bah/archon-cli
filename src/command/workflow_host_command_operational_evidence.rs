use super::OperationalAttempt;
use crate::command::workflow_host_secrets::HostSecrets;

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
pub(crate) fn stderr_tail(stderr: &[u8], secrets: &HostSecrets) -> String {
    const MAX_BYTES: usize = 4096;
    let decoded = String::from_utf8_lossy(stderr);
    let configured_redacted = secrets.text(&decoded);
    let redacted = archon_observability::redaction::redact_secret_values(&configured_redacted);
    let mut start = redacted.len().saturating_sub(MAX_BYTES);
    while !redacted.is_char_boundary(start) {
        start += 1;
    }
    redacted[start..].to_string()
}
