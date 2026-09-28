//! Captured check output: the inline tail a round record keeps, and the
//! full output written beside it.

use std::path::Path;

use archon_workflow::acceptance_scratch::CheckResult;
use archon_workflow::failure_evidence::failure_evidence;

/// Bytes of stdout/stderr kept inline in the round record, and handed to a
/// remediation agent verbatim; the full captured output (bounded by the
/// site's output limit) is written beside it.
const OUTPUT_TAIL_BYTES: usize = 4000;

/// A stream's failure evidence: its end and every line stating a failure,
/// bounded (`archon_workflow::failure_evidence`). Never a bare byte tail: a
/// long `cargo run` prints its one `Error:` line after pages of warnings.
pub(super) fn tail(bytes: &[u8]) -> String {
    failure_evidence(bytes, OUTPUT_TAIL_BYTES)
}

/// The short form a command record's `output_summary` carries.
pub(super) fn brief(evidence: &str) -> String {
    failure_evidence(evidence.as_bytes(), 400)
}

pub(super) fn write_output_files(dir: &Path, result: &CheckResult) {
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let safe: String = result
        .acceptance_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect();
    let _ = std::fs::write(dir.join(format!("{safe}.stdout")), &result.stdout);
    let _ = std::fs::write(dir.join(format!("{safe}.stderr")), &result.stderr);
}
