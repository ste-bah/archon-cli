//! World-model storage facade.
//!
//! The PRD calls for a split store: JSONL for append-only audit evidence and
//! Cozo for indexed joins/evaluation. This module keeps that contract small.

pub mod cozo_store;
pub mod jsonl;
pub mod retention;

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::Result;
use cozo::DbInstance;

use crate::schema::WorldTraceRow;
use crate::trace::ColdStartStats;
pub use retention::{RetentionPolicy, RetentionReport};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistSummary {
    pub jsonl_rows: usize,
    pub cozo_rows: usize,
    pub ledger_path: PathBuf,
    pub db_path: PathBuf,
}

pub struct WorldModelStore {
    root: PathBuf,
    db_path: PathBuf,
    db: DbInstance,
}

impl WorldModelStore {
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        // Ensure root exists before canonicalizing; required on macOS where
        // tempfile::tempdir() returns /var/folders/... that resolves to
        // /private/var/folders/.... Without canonicalization, repeated
        // open(root) calls would key the sqlite-backed Cozo instance under
        // two different effective paths and silently see empty state.
        std::fs::create_dir_all(root.as_ref())?;
        let root = std::fs::canonicalize(root.as_ref())?;
        std::fs::create_dir_all(root.join("ledgers"))?;

        let db_path = root.join("world-model.db");
        let db = DbInstance::new("sqlite", &db_path, "")
            .map_err(|e| anyhow::anyhow!("open world-model Cozo store failed: {e}"))?;
        cozo_store::ensure_schema(&db)?;

        Ok(Self { root, db_path, db })
    }

    pub fn persist_rows(&self, rows: &[WorldTraceRow]) -> Result<PersistSummary> {
        self.persist_rows_with_retention(rows, RetentionPolicy::default())
    }

    pub fn persist_rows_with_retention(
        &self,
        rows: &[WorldTraceRow],
        policy: RetentionPolicy,
    ) -> Result<PersistSummary> {
        retention::apply_retention(&self.root, policy)?;
        let ledger_path = jsonl::append_rows(&self.root, rows)?;
        let cozo_rows = cozo_store::put_rows(&self.db, rows)?;

        Ok(PersistSummary {
            jsonl_rows: rows.len(),
            cozo_rows,
            ledger_path,
            db_path: self.db_path.clone(),
        })
    }

    pub fn cold_start_stats(&self) -> Result<ColdStartStats> {
        cozo_store::cold_start_stats(&self.db)
    }

    pub fn load_rows(&self) -> Result<Vec<WorldTraceRow>> {
        cozo_store::all_rows(&self.db)
    }

    pub fn row_ids(&self) -> Result<HashSet<String>> {
        cozo_store::row_ids(&self.db)
    }
}

#[cfg(test)]
mod tests {
    use crate::schema::{WorldActionKind, WorldTraceRow};

    use super::*;

    #[test]
    fn store_persists_rows_to_jsonl_and_cozo() {
        let temp = tempfile::tempdir().unwrap();
        let store = WorldModelStore::open(temp.path()).unwrap();
        let row = WorldTraceRow::new("session-1", WorldActionKind::ToolCall).with_row_id("row-1");

        let summary = store.persist_rows(&[row]).unwrap();

        assert_eq!(summary.jsonl_rows, 1);
        assert_eq!(summary.cozo_rows, 1);
        assert!(summary.ledger_path.exists());
        assert!(summary.db_path.exists());
        assert_eq!(cozo_store::count_rows(&store.db).unwrap(), 1);
        assert_eq!(store.cold_start_stats().unwrap().rows, 1);
    }

    #[test]
    fn raw_ledger_rotation_keeps_cozo_summaries() {
        let temp = tempfile::tempdir().unwrap();
        let store = WorldModelStore::open(temp.path()).unwrap();
        let row = WorldTraceRow::new("session-1", WorldActionKind::ToolCall).with_row_id("row-1");
        store.persist_rows(&[row]).unwrap();

        retention::apply_retention(
            temp.path(),
            RetentionPolicy {
                jsonl_rotate_bytes: 1,
                raw_retention_days: 90,
            },
        )
        .unwrap();

        assert_eq!(store.cold_start_stats().unwrap().rows, 1);
    }

    #[test]
    fn store_loads_persisted_rows() {
        let temp = tempfile::tempdir().unwrap();
        let store = WorldModelStore::open(temp.path()).unwrap();
        let row = WorldTraceRow::new("session-1", WorldActionKind::ToolCall).with_row_id("row-1");

        store.persist_rows(&[row]).unwrap();

        let rows = store.load_rows().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].row_id, "row-1");
        assert!(store.row_ids().unwrap().contains("row-1"));
    }
}
