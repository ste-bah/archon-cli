use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use cozo::{DataValue, DbInstance, NamedRows, ScriptMutability};

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum CheckpointError {
    #[error("checkpoint database error: {0}")]
    DbError(String),

    #[error("checkpoint I/O error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("snapshot not found for path: {0}")]
    NotFound(String),
}

fn db_err(e: impl std::fmt::Display) -> CheckpointError {
    CheckpointError::DbError(e.to_string())
}

#[cfg(unix)]
fn secure_file_permissions(path: &std::path::Path) -> Result<(), std::io::Error> {
    use std::os::unix::fs::PermissionsExt;
    let perms = std::fs::Permissions::from_mode(0o600);
    std::fs::set_permissions(path, perms)
}

fn empty_rows() -> NamedRows {
    NamedRows::new(vec![], vec![])
}

fn extract_str(val: &DataValue) -> String {
    val.get_str().unwrap_or("").to_string()
}

fn extract_i64(val: &DataValue) -> i64 {
    val.get_int().unwrap_or(0)
}

fn extract_bool_from_int(val: &DataValue) -> bool {
    extract_i64(val) != 0
}

fn extract_bytes(val: &DataValue) -> Option<Vec<u8>> {
    val.get_bytes().map(|b| b.to_vec())
}

// ---------------------------------------------------------------------------
// Snapshot info
// ---------------------------------------------------------------------------

/// Metadata about a checkpointed file.
#[derive(Debug, Clone)]
pub struct SnapshotInfo {
    pub file_path: String,
    pub turn_number: i64,
    pub file_existed: bool,
    pub tool_name: String,
    pub timestamp: String,
}

// ---------------------------------------------------------------------------
// Checkpoint store
// ---------------------------------------------------------------------------

/// Stores file snapshots so that modified files can be restored to their
/// original content. Backed by a CozoDB database.
pub struct CheckpointStore {
    db: DbInstance,
    max_snapshots: u32,
}

const DEFAULT_MAX_SNAPSHOTS: u32 = 500;

impl CheckpointStore {
    /// Open (or create) a checkpoint database at the given path.
    pub fn open(path: &Path) -> Result<Self, CheckpointError> {
        Self::open_with_limit(path, DEFAULT_MAX_SNAPSHOTS)
    }

    /// Open with a custom snapshot limit per session.
    pub fn open_with_limit(path: &Path, max_snapshots: u32) -> Result<Self, CheckpointError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let path_str = path.to_string_lossy().to_string();
        let db = DbInstance::new("sqlite", &path_str, "").map_err(db_err)?;

        #[cfg(unix)]
        secure_file_permissions(path)?;

        let store = Self { db, max_snapshots };
        store.init_schema()?;
        Ok(store)
    }

    /// Create an in-memory checkpoint store (useful for tests).
    #[cfg(test)]
    fn in_memory() -> Result<Self, CheckpointError> {
        Self::in_memory_with_limit(DEFAULT_MAX_SNAPSHOTS)
    }

    /// Create an in-memory checkpoint store with a custom limit.
    #[cfg(test)]
    fn in_memory_with_limit(max_snapshots: u32) -> Result<Self, CheckpointError> {
        let db = DbInstance::new("mem", "", "").map_err(db_err)?;
        let store = Self { db, max_snapshots };
        store.init_schema()?;
        Ok(store)
    }

    fn init_schema(&self) -> Result<(), CheckpointError> {
        self.db
            .run_script(
                ":create checkpoints {
                    session_id: String,
                    file_path: String,
                    turn_number: Int
                    =>
                    original_content: Bytes,
                    file_existed: Int,
                    tool_name: String,
                    timestamp: String
                }",
                Default::default(),
                ScriptMutability::Mutable,
            )
            .or_else(|e| {
                let msg = e.to_string();
                if msg.contains("already exists") || msg.contains("conflicts") {
                    Ok(empty_rows())
                } else {
                    Err(db_err(e))
                }
            })?;

        Ok(())
    }

    /// Snapshot a file before it is modified.
    pub fn snapshot(
        &self,
        session_id: &str,
        file_path: &str,
        turn_number: i64,
        tool_name: &str,
    ) -> Result<(), CheckpointError> {
        let path = Path::new(file_path);
        let (content, existed) = if path.exists() {
            (fs::read(path)?, true)
        } else {
            (Vec::new(), false)
        };

        let now = chrono::Utc::now().to_rfc3339();

        let mut params = BTreeMap::new();
        params.insert("session_id".to_string(), DataValue::from(session_id));
        params.insert("file_path".to_string(), DataValue::from(file_path));
        params.insert("turn_number".to_string(), DataValue::from(turn_number));
        params.insert("original_content".to_string(), DataValue::from(content));
        params.insert(
            "file_existed".to_string(),
            DataValue::from(if existed { 1i64 } else { 0i64 }),
        );
        params.insert("tool_name".to_string(), DataValue::from(tool_name));
        params.insert("timestamp".to_string(), DataValue::from(now.as_str()));

        self.db
            .run_script(
                "?[session_id, file_path, turn_number, original_content, file_existed, tool_name, timestamp] <- [[
                    $session_id, $file_path, $turn_number,
                    $original_content, $file_existed, $tool_name, $timestamp
                ]]
                :put checkpoints {
                    session_id, file_path, turn_number
                    => original_content, file_existed, tool_name, timestamp
                }",
                params,
                ScriptMutability::Mutable,
            )
            .map_err(db_err)?;

        // LRU eviction
        self.evict(session_id)?;

        Ok(())
    }

    /// Restore a file to its most recently snapshotted content.
    pub fn restore(&self, session_id: &str, file_path: &str) -> Result<(), CheckpointError> {
        let mut params = BTreeMap::new();
        params.insert("sid".to_string(), DataValue::from(session_id));
        params.insert("fp".to_string(), DataValue::from(file_path));

        let result = self
            .db
            .run_script(
                "?[original_content, file_existed, turn_number] :=
                    *checkpoints{session_id, file_path, turn_number, original_content, file_existed},
                    session_id = $sid, file_path = $fp
                :sort -turn_number
                :limit 1",
                params,
                ScriptMutability::Immutable,
            )
            .map_err(db_err)?;

        if result.rows.is_empty() {
            return Err(CheckpointError::NotFound(file_path.to_string()));
        }

        let row = &result.rows[0];
        let content = extract_bytes(&row[0]);
        let existed = extract_bool_from_int(&row[1]);

        let target = Path::new(file_path);
        if existed {
            if let Some(data) = content
                && !data.is_empty()
            {
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(target, data)?;
            }
        } else {
            if target.exists() {
                fs::remove_file(target)?;
            }
        }

        Ok(())
    }

    /// List all files that have been snapshotted in a session.
    pub fn list_modified(&self, session_id: &str) -> Result<Vec<SnapshotInfo>, CheckpointError> {
        let mut params = BTreeMap::new();
        params.insert("sid".to_string(), DataValue::from(session_id));

        let result = self
            .db
            .run_script(
                "?[file_path, turn_number, file_existed, tool_name, timestamp] :=
                    *checkpoints{session_id, file_path, turn_number, file_existed, tool_name, timestamp},
                    session_id = $sid
                :sort turn_number",
                params,
                ScriptMutability::Immutable,
            )
            .map_err(db_err)?;

        let mut snapshots = Vec::new();
        for row in &result.rows {
            snapshots.push(SnapshotInfo {
                file_path: extract_str(&row[0]),
                turn_number: extract_i64(&row[1]),
                file_existed: extract_bool_from_int(&row[2]),
                tool_name: extract_str(&row[3]),
                timestamp: extract_str(&row[4]),
            });
        }

        Ok(snapshots)
    }

    /// Get the snapshotted content for a specific file at a specific turn.
    ///
    /// Returns the content as a UTF-8 string. Binary content is returned as
    /// lossy UTF-8.
    pub fn get_content(
        &self,
        session_id: &str,
        file_path: &str,
        turn_number: i64,
    ) -> Result<String, CheckpointError> {
        let mut params = BTreeMap::new();
        params.insert("sid".to_string(), DataValue::from(session_id));
        params.insert("fp".to_string(), DataValue::from(file_path));
        params.insert("turn".to_string(), DataValue::from(turn_number));

        let result = self
            .db
            .run_script(
                "?[original_content] :=
                    *checkpoints{session_id, file_path, turn_number, original_content},
                    session_id = $sid, file_path = $fp, turn_number = $turn",
                params,
                ScriptMutability::Immutable,
            )
            .map_err(db_err)?;

        if result.rows.is_empty() {
            return Err(CheckpointError::NotFound(format!(
                "{file_path} at turn {turn_number}"
            )));
        }

        let content_bytes = extract_bytes(&result.rows[0][0]).unwrap_or_default();
        Ok(String::from_utf8_lossy(&content_bytes).into_owned())
    }

    /// Generate a unified diff between a checkpoint snapshot and the current file content.
    /// Returns the diff as a string (empty if files are identical).
    pub fn diff(
        &self,
        session_id: &str,
        file_path: &str,
        turn_number: i64,
    ) -> Result<String, CheckpointError> {
        let snapshot_content = self.get_content(session_id, file_path, turn_number)?;
        let current_content = fs::read_to_string(file_path).unwrap_or_default();
        Ok(generate_unified_diff(
            file_path,
            &snapshot_content,
            &current_content,
        ))
    }

    /// Restore a file to a specific turn's snapshot (not just the latest).
    pub fn restore_to_turn(
        &self,
        session_id: &str,
        file_path: &str,
        turn_number: i64,
    ) -> Result<(), CheckpointError> {
        let mut params = BTreeMap::new();
        params.insert("sid".to_string(), DataValue::from(session_id));
        params.insert("fp".to_string(), DataValue::from(file_path));
        params.insert("turn".to_string(), DataValue::from(turn_number));

        let result = self
            .db
            .run_script(
                "?[original_content, file_existed] :=
                    *checkpoints{session_id, file_path, turn_number, original_content, file_existed},
                    session_id = $sid, file_path = $fp, turn_number = $turn",
                params,
                ScriptMutability::Immutable,
            )
            .map_err(db_err)?;

        if result.rows.is_empty() {
            return Err(CheckpointError::NotFound(format!(
                "{file_path} at turn {turn_number}"
            )));
        }

        let row = &result.rows[0];
        let content = extract_bytes(&row[0]);
        let existed = extract_bool_from_int(&row[1]);

        let target = Path::new(file_path);
        if existed {
            if let Some(data) = content
                && !data.is_empty()
            {
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(target, data)?;
            }
        } else {
            if target.exists() {
                fs::remove_file(target)?;
            }
        }

        Ok(())
    }

    /// Evict oldest snapshots when the session exceeds `max_snapshots`.
    fn evict(&self, session_id: &str) -> Result<(), CheckpointError> {
        let mut params = BTreeMap::new();
        params.insert("sid".to_string(), DataValue::from(session_id));

        // Count snapshots for this session
        let count_result = self
            .db
            .run_script(
                "?[count(file_path)] := *checkpoints{session_id, file_path}, session_id = $sid",
                params.clone(),
                ScriptMutability::Immutable,
            )
            .map_err(db_err)?;

        let count = if count_result.rows.is_empty() {
            0i64
        } else {
            extract_i64(&count_result.rows[0][0])
        };

        if count > self.max_snapshots as i64 {
            let excess = count - self.max_snapshots as i64;

            // Find the oldest entries
            let oldest = self
                .db
                .run_script(
                    "?[session_id, file_path, turn_number] :=
                        *checkpoints{session_id, file_path, turn_number},
                        session_id = $sid
                    :sort turn_number",
                    params,
                    ScriptMutability::Immutable,
                )
                .map_err(db_err)?;

            for (i, row) in oldest.rows.iter().enumerate() {
                if i as i64 >= excess {
                    break;
                }
                let mut del_params = BTreeMap::new();
                del_params.insert("session_id".to_string(), row[0].clone());
                del_params.insert("file_path".to_string(), row[1].clone());
                del_params.insert("turn_number".to_string(), row[2].clone());

                self.db
                    .run_script(
                        "?[session_id, file_path, turn_number] <- [[$session_id, $file_path, $turn_number]]
                         :rm checkpoints {session_id, file_path, turn_number}",
                        del_params,
                        ScriptMutability::Mutable,
                    )
                    .map_err(db_err)?;
            }
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Unified diff helpers (no external crate needed)
// ---------------------------------------------------------------------------

/// Generate a simple unified diff between two strings.
fn generate_unified_diff(filename: &str, old: &str, new: &str) -> String {
    if old == new {
        return String::new();
    }

    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();

    let mut output = String::new();
    output.push_str(&format!("--- a/{filename}\n"));
    output.push_str(&format!("+++ b/{filename}\n"));
    output.push_str(&format!(
        "@@ -1,{} +1,{} @@\n",
        old_lines.len(),
        new_lines.len()
    ));

    let lcs = compute_lcs(&old_lines, &new_lines);
    let mut oi = 0;
    let mut ni = 0;
    let mut li = 0;

    while oi < old_lines.len() || ni < new_lines.len() {
        if li < lcs.len()
            && oi < old_lines.len()
            && ni < new_lines.len()
            && old_lines[oi] == lcs[li]
            && new_lines[ni] == lcs[li]
        {
            output.push_str(&format!(" {}\n", old_lines[oi]));
            oi += 1;
            ni += 1;
            li += 1;
        } else {
            if oi < old_lines.len() && (li >= lcs.len() || old_lines[oi] != lcs[li]) {
                output.push_str(&format!("-{}\n", old_lines[oi]));
                oi += 1;
            }
            if ni < new_lines.len() && (li >= lcs.len() || new_lines[ni] != lcs[li]) {
                output.push_str(&format!("+{}\n", new_lines[ni]));
                ni += 1;
            }
        }
    }

    output
}

fn compute_lcs<'a>(a: &[&'a str], b: &[&'a str]) -> Vec<&'a str> {
    let m = a.len();
    let n = b.len();
    let mut dp = vec![vec![0usize; n + 1]; m + 1];

    for i in 1..=m {
        for j in 1..=n {
            if a[i - 1] == b[j - 1] {
                dp[i][j] = dp[i - 1][j - 1] + 1;
            } else {
                dp[i][j] = dp[i - 1][j].max(dp[i][j - 1]);
            }
        }
    }

    let mut result = Vec::new();
    let mut i = m;
    let mut j = n;
    while i > 0 && j > 0 {
        if a[i - 1] == b[j - 1] {
            result.push(a[i - 1]);
            i -= 1;
            j -= 1;
        } else if dp[i - 1][j] > dp[i][j - 1] {
            i -= 1;
        } else {
            j -= 1;
        }
    }
    result.reverse();
    result
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    fn temp_db() -> (tempfile::TempDir, CheckpointStore) {
        let dir = tempfile::tempdir().expect("tempdir failed");
        let db_path = dir.path().join("checkpoints.db");
        let store = CheckpointStore::open(&db_path).expect("open failed");
        (dir, store)
    }

    fn temp_db_with_limit(limit: u32) -> (tempfile::TempDir, CheckpointStore) {
        let dir = tempfile::tempdir().expect("tempdir failed");
        let db_path = dir.path().join("checkpoints.db");
        let store = CheckpointStore::open_with_limit(&db_path, limit).expect("open failed");
        (dir, store)
    }

    #[test]
    fn roundtrip_existing_file() {
        let (dir, store) = temp_db();

        let file_path = dir.path().join("test.txt");
        fs::write(&file_path, b"original content").expect("write failed");
        let fp = file_path.to_str().expect("path to str failed");

        store
            .snapshot("sess1", fp, 1, "write_file")
            .expect("snapshot failed");

        fs::write(&file_path, b"modified content").expect("write failed");
        assert_eq!(
            fs::read_to_string(&file_path).expect("read failed"),
            "modified content"
        );

        store.restore("sess1", fp).expect("restore failed");
        assert_eq!(
            fs::read_to_string(&file_path).expect("read failed"),
            "original content"
        );
    }

    #[test]
    fn new_file_deletion_on_restore() {
        let (dir, store) = temp_db();

        let file_path = dir.path().join("new_file.txt");
        let fp = file_path.to_str().expect("path to str failed");

        store
            .snapshot("sess1", fp, 1, "create_file")
            .expect("snapshot failed");

        fs::write(&file_path, b"new content").expect("write failed");
        assert!(file_path.exists());

        store.restore("sess1", fp).expect("restore failed");
        assert!(!file_path.exists());
    }

    #[test]
    fn lru_eviction() {
        let (dir, store) = temp_db_with_limit(3);

        for i in 0..5 {
            let fp = dir.path().join(format!("file_{i}.txt"));
            fs::write(&fp, format!("content {i}")).expect("write failed");
            store
                .snapshot("sess1", fp.to_str().expect("path"), i, "write")
                .expect("snapshot failed");
        }

        let modified = store.list_modified("sess1").expect("list failed");
        assert_eq!(modified.len(), 3);
        assert!(modified.iter().all(|s| s.turn_number >= 2));
    }

    #[test]
    fn list_modified_files() {
        let (dir, store) = temp_db();

        let f1 = dir.path().join("a.txt");
        let f2 = dir.path().join("b.txt");
        fs::write(&f1, "a").expect("write failed");
        fs::write(&f2, "b").expect("write failed");

        store
            .snapshot("sess1", f1.to_str().expect("path"), 1, "edit")
            .expect("snapshot failed");
        store
            .snapshot("sess1", f2.to_str().expect("path"), 2, "write")
            .expect("snapshot failed");

        let list = store.list_modified("sess1").expect("list failed");
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].tool_name, "edit");
        assert_eq!(list[1].tool_name, "write");
    }

    #[test]
    fn missing_snapshot_returns_error() {
        let (_dir, store) = temp_db();
        let result = store.restore("sess1", "/nonexistent/path.txt");
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), CheckpointError::NotFound(_)));
    }

    #[test]
    fn session_isolation() {
        let (dir, store) = temp_db();

        let fp = dir.path().join("shared.txt");
        fs::write(&fp, "v1").expect("write failed");
        let fp_str = fp.to_str().expect("path");

        store
            .snapshot("sess_a", fp_str, 1, "write")
            .expect("snapshot failed");

        fs::write(&fp, "v2").expect("write failed");
        store
            .snapshot("sess_b", fp_str, 1, "write")
            .expect("snapshot failed");

        store.restore("sess_a", fp_str).expect("restore failed");
        assert_eq!(fs::read_to_string(&fp).expect("read failed"), "v1");

        store.restore("sess_b", fp_str).expect("restore failed");
        assert_eq!(fs::read_to_string(&fp).expect("read failed"), "v2");
    }

    #[test]
    fn binary_content_roundtrip() {
        let (dir, store) = temp_db();

        let fp = dir.path().join("binary.bin");
        let binary_data: Vec<u8> = (0..=255).collect();
        {
            let mut f = fs::File::create(&fp).expect("create failed");
            f.write_all(&binary_data).expect("write failed");
        }
        let fp_str = fp.to_str().expect("path");

        store
            .snapshot("sess1", fp_str, 1, "write")
            .expect("snapshot failed");

        fs::write(&fp, b"replaced").expect("write failed");

        store.restore("sess1", fp_str).expect("restore failed");
        assert_eq!(fs::read(&fp).expect("read failed"), binary_data);
    }

    #[test]
    fn get_content_returns_snapshot() {
        let (dir, store) = temp_db();
        let fp = dir.path().join("content.txt");
        fs::write(&fp, "snapshot content").expect("write failed");
        let fp_str = fp.to_str().expect("path");
        store
            .snapshot("sess1", fp_str, 1, "edit")
            .expect("snapshot");
        let content = store.get_content("sess1", fp_str, 1).expect("get_content");
        assert_eq!(content, "snapshot content");
    }

    #[test]
    fn get_content_not_found() {
        let (_dir, store) = temp_db();
        let result = store.get_content("sess1", "/nope.txt", 99);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), CheckpointError::NotFound(_)));
    }

    #[test]
    fn multiple_turns_restores_latest() {
        let (dir, store) = temp_db();

        let fp = dir.path().join("multi.txt");
        fs::write(&fp, "turn1").expect("write failed");
        let fp_str = fp.to_str().expect("path");
        store
            .snapshot("sess1", fp_str, 1, "write")
            .expect("snapshot failed");

        fs::write(&fp, "turn2").expect("write failed");
        store
            .snapshot("sess1", fp_str, 2, "write")
            .expect("snapshot failed");

        fs::write(&fp, "turn3_modified").expect("write failed");

        store.restore("sess1", fp_str).expect("restore failed");
        assert_eq!(fs::read_to_string(&fp).expect("read failed"), "turn2");
    }

    #[test]
    fn diff_shows_changes() {
        let (dir, store) = temp_db();
        let fp = dir.path().join("diff_test.txt");
        fs::write(&fp, "line1\nline2\nline3\n").expect("write");
        let fp_str = fp.to_str().expect("path");

        store
            .snapshot("sess1", fp_str, 1, "write")
            .expect("snapshot");
        fs::write(&fp, "line1\nmodified\nline3\n").expect("write");

        let diff = store.diff("sess1", fp_str, 1).expect("diff");
        assert!(diff.contains("-line2"), "diff should contain removed line");
        assert!(diff.contains("+modified"), "diff should contain added line");
    }

    #[test]
    fn diff_empty_when_unchanged() {
        let (dir, store) = temp_db();
        let fp = dir.path().join("same.txt");
        fs::write(&fp, "same content\n").expect("write");
        let fp_str = fp.to_str().expect("path");

        store
            .snapshot("sess1", fp_str, 1, "write")
            .expect("snapshot");
        // Don't modify the file

        let diff = store.diff("sess1", fp_str, 1).expect("diff");
        assert!(diff.is_empty(), "diff should be empty for unchanged file");
    }

    #[test]
    fn restore_to_specific_turn() {
        let (dir, store) = temp_db();
        let fp = dir.path().join("turns.txt");

        fs::write(&fp, "version1").expect("write");
        let fp_str = fp.to_str().expect("path");
        store
            .snapshot("sess1", fp_str, 1, "write")
            .expect("snapshot");

        fs::write(&fp, "version2").expect("write");
        store
            .snapshot("sess1", fp_str, 2, "write")
            .expect("snapshot");

        fs::write(&fp, "version3").expect("write");

        // Restore to turn 1 (should get "version1")
        store.restore_to_turn("sess1", fp_str, 1).expect("restore");
        assert_eq!(fs::read_to_string(&fp).expect("read"), "version1");
    }
}
