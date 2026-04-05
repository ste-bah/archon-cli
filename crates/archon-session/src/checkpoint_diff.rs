//! Checkpoint diff and selective restore utilities.
//!
//! Provides a simple line-by-line diff algorithm that produces unified diff
//! output, plus data types for listing and restoring file checkpoints.

use std::fmt::Write;

// ---------------------------------------------------------------------------
// CheckpointEntry
// ---------------------------------------------------------------------------

/// Metadata about a single file checkpoint.
#[derive(Debug, Clone)]
pub struct CheckpointEntry {
    /// Unique identifier for this checkpoint.
    pub checkpoint_id: String,
    /// Absolute path of the file that was checkpointed.
    pub file_path: String,
    /// ISO-8601 timestamp of when the checkpoint was created.
    pub timestamp: String,
    /// Size of the file content in bytes at this checkpoint.
    pub size_bytes: usize,
}

// ---------------------------------------------------------------------------
// Unified diff
// ---------------------------------------------------------------------------

/// Compare two file snapshots and produce a unified diff string.
///
/// Uses a longest-common-subsequence (LCS) algorithm to compute line-level
/// differences. Output follows unified diff format.
///
/// If the two snapshots are identical the returned string contains only the
/// file headers (no hunks).
pub fn diff_snapshots(before: &str, after: &str, filename: &str) -> String {
    let before_lines: Vec<&str> = if before.is_empty() {
        Vec::new()
    } else {
        before.lines().collect()
    };
    let after_lines: Vec<&str> = if after.is_empty() {
        Vec::new()
    } else {
        after.lines().collect()
    };

    let ops = compute_edit_script(&before_lines, &after_lines);

    let has_changes = ops.iter().any(|op| !matches!(op, EditOp::Keep(_, _)));
    if !has_changes {
        return format!("--- a/{filename}\n+++ b/{filename}\n");
    }

    let mut out = String::new();
    let _ = writeln!(out, "--- a/{filename}");
    let _ = writeln!(out, "+++ b/{filename}");

    // Build a single hunk covering all operations.
    let mut old_count = 0usize;
    let mut new_count = 0usize;
    let mut lines: Vec<String> = Vec::new();

    for op in &ops {
        match op {
            EditOp::Keep(oi, _ni) => {
                lines.push(format!(" {}", before_lines[*oi]));
                old_count += 1;
                new_count += 1;
            }
            EditOp::Remove(oi) => {
                lines.push(format!("-{}", before_lines[*oi]));
                old_count += 1;
            }
            EditOp::Add(ni) => {
                lines.push(format!("+{}", after_lines[*ni]));
                new_count += 1;
            }
        }
    }

    let old_start = if old_count == 0 { 0 } else { 1 };
    let new_start = if new_count == 0 { 0 } else { 1 };

    let _ = writeln!(
        out,
        "@@ -{old_start},{old_count} +{new_start},{new_count} @@"
    );
    for line in &lines {
        let _ = writeln!(out, "{line}");
    }

    out
}

/// List all checkpoints for a given file path.
pub fn list_file_checkpoints(store: &[CheckpointEntry], file_path: &str) -> Vec<CheckpointEntry> {
    store
        .iter()
        .filter(|e| e.file_path == file_path)
        .cloned()
        .collect()
}

/// Restore a file to a specific checkpoint.
///
/// Looks up the checkpoint by ID and file path in the provided store and
/// returns the content string. Returns `Err` if not found.
pub fn restore_checkpoint(
    store: &[(CheckpointEntry, String)],
    checkpoint_id: &str,
    file_path: &str,
) -> Result<String, String> {
    for (entry, content) in store {
        if entry.checkpoint_id == checkpoint_id && entry.file_path == file_path {
            return Ok(content.clone());
        }
    }
    Err(format!(
        "checkpoint '{checkpoint_id}' not found for file '{file_path}'"
    ))
}

// ---------------------------------------------------------------------------
// Internal: edit script via LCS
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
enum EditOp {
    Keep(usize, usize), // (old_idx, new_idx)
    Remove(usize),      // old_idx
    Add(usize),         // new_idx
}

/// Compute a minimal edit script using LCS dynamic programming.
fn compute_edit_script(before: &[&str], after: &[&str]) -> Vec<EditOp> {
    let m = before.len();
    let n = after.len();

    // Build LCS table
    let mut table = vec![vec![0u32; n + 1]; m + 1];
    for i in 1..=m {
        for j in 1..=n {
            if before[i - 1] == after[j - 1] {
                table[i][j] = table[i - 1][j - 1] + 1;
            } else {
                table[i][j] = table[i - 1][j].max(table[i][j - 1]);
            }
        }
    }

    // Backtrack to produce edit operations
    let mut ops = Vec::new();
    let mut i = m;
    let mut j = n;

    while i > 0 || j > 0 {
        if i > 0 && j > 0 && before[i - 1] == after[j - 1] {
            ops.push(EditOp::Keep(i - 1, j - 1));
            i -= 1;
            j -= 1;
        } else if j > 0 && (i == 0 || table[i][j - 1] >= table[i - 1][j]) {
            ops.push(EditOp::Add(j - 1));
            j -= 1;
        } else {
            ops.push(EditOp::Remove(i - 1));
            i -= 1;
        }
    }

    ops.reverse();
    ops
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lcs_identical() {
        let a = vec!["x", "y"];
        let b = vec!["x", "y"];
        let ops = compute_edit_script(&a, &b);
        assert!(ops.iter().all(|o| matches!(o, EditOp::Keep(_, _))));
    }

    #[test]
    fn lcs_all_added() {
        let a: Vec<&str> = vec![];
        let b = vec!["a", "b"];
        let ops = compute_edit_script(&a, &b);
        assert_eq!(ops.len(), 2);
        assert!(ops.iter().all(|o| matches!(o, EditOp::Add(_))));
    }

    #[test]
    fn list_file_checkpoints_filters() {
        let entries = vec![
            CheckpointEntry {
                checkpoint_id: "1".into(),
                file_path: "/a.rs".into(),
                timestamp: "t1".into(),
                size_bytes: 10,
            },
            CheckpointEntry {
                checkpoint_id: "2".into(),
                file_path: "/b.rs".into(),
                timestamp: "t2".into(),
                size_bytes: 20,
            },
        ];
        let filtered = list_file_checkpoints(&entries, "/a.rs");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].checkpoint_id, "1");
    }

    #[test]
    fn restore_found() {
        let store = vec![(
            CheckpointEntry {
                checkpoint_id: "cp1".into(),
                file_path: "/main.rs".into(),
                timestamp: "t".into(),
                size_bytes: 5,
            },
            "fn main() {}".to_string(),
        )];
        let result = restore_checkpoint(&store, "cp1", "/main.rs");
        assert_eq!(result, Ok("fn main() {}".to_string()));
    }

    #[test]
    fn restore_not_found() {
        let store: Vec<(CheckpointEntry, String)> = vec![];
        let result = restore_checkpoint(&store, "missing", "/x.rs");
        assert!(result.is_err());
    }
}
