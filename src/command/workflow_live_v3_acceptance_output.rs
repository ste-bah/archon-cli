//! Captured check output: the inline tail a round record keeps, and the
//! full output written beside it.

use std::path::Path;

use archon_workflow::acceptance_scratch::CheckResult;

/// Bytes of stdout/stderr kept inline in the round record; the full captured
/// output (bounded by the site's output limit) is written beside it.
const OUTPUT_TAIL_BYTES: usize = 4000;

pub(super) fn tail(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    if text.len() <= OUTPUT_TAIL_BYTES {
        return text.into_owned();
    }
    let mut start = text.len() - OUTPUT_TAIL_BYTES;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    format!("[truncated]\n{}", &text[start..])
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
