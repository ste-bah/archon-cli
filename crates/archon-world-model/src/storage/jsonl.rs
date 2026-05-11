//! Append-only JSONL ledger for world-model trace rows.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::schema::WorldTraceRow;

pub fn ledger_path(root: &Path) -> PathBuf {
    root.join("ledgers").join("world-trace-rows.jsonl")
}

pub fn append_rows(root: &Path, rows: &[WorldTraceRow]) -> Result<PathBuf> {
    let path = ledger_path(root);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
    for row in rows {
        serde_json::to_writer(&mut file, row)?;
        file.write_all(b"\n")?;
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use crate::schema::{WorldActionKind, WorldTraceRow};

    use super::*;

    #[test]
    fn appends_one_json_line_per_row() {
        let temp = tempfile::tempdir().unwrap();
        let rows = [
            WorldTraceRow::new("s1", WorldActionKind::ToolCall).with_row_id("r1"),
            WorldTraceRow::new("s1", WorldActionKind::Retry).with_row_id("r2"),
        ];

        let path = append_rows(temp.path(), &rows).unwrap();
        let content = fs::read_to_string(path).unwrap();

        assert_eq!(content.lines().count(), 2);
        assert!(content.contains("\"row_id\":\"r1\""));
        assert!(content.contains("\"row_id\":\"r2\""));
    }
}
