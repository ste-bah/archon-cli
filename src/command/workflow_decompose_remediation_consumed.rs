//! The consumed-report journal: which reverify reports a resume already
//! consumed. Rows are appended; an interrupted append is never read as one.

use super::*;
use std::io::Write;

pub(crate) fn is_consumed(run_dir: &Path, report_id: &str) -> Result<bool> {
    let path = run_dir.join("decomposition/reverify/consumed.jsonl");
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
    };
    for row in super::resolved::resolved_rows(&text, &path)? {
        if row["report_id"].as_str() == Some(report_id) {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(crate) fn append_consumed(run_dir: &Path, ledger: &Ledger) -> Result<()> {
    let path = run_dir.join("decomposition/reverify/consumed.jsonl");
    std::fs::create_dir_all(path.parent().expect("ledger parent"))?;
    // An interrupted append leaves an unterminated fragment the readers
    // ignore; cut it, so this row starts its own line instead of merging
    // with it into one terminated, malformed line.
    let existing = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
    };
    if existing.last().is_some_and(|byte| *byte != b'\n') {
        let kept = existing
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |at| at + 1);
        let torn = std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .with_context(|| format!("opening {}", path.display()))?;
        torn.set_len(kept as u64)
            .and_then(|()| torn.sync_all())
            .with_context(|| format!("cutting the torn tail of {}", path.display()))?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("opening {}", path.display()))?;
    let mut row = Vec::new();
    serde_json::to_writer(
        &mut row,
        &serde_json::json!({
            "report_id": ledger.report_id,
            "subjects": ledger.subjects,
            "consumed_at": chrono::Utc::now(),
        }),
    )?;
    row.push(b'\n');
    file.write_all(&row)?;
    file.sync_all()?;
    Ok(())
}
