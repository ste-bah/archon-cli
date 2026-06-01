//! World-model storage facade.
//!
//! The PRD calls for a split store: JSONL for append-only audit evidence and
//! Cozo for indexed joins/evaluation. This module keeps that contract small.

pub mod cozo_store;
pub mod deferred_retry;
pub mod jsonl;
pub mod retention;

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use cozo::{DbInstance, ScriptMutability};

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

const COZO_LOCK_RETRY_DELAYS_MS: [u64; 7] = [25, 50, 100, 200, 400, 800, 1_600];

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
        let config = cozo_config(&db_path);
        let db_path_str = db_path.to_string_lossy().to_string();
        let db =
            archon_cozo::open_sqlite_guarded(&db_path_str, "open world-model Cozo store", &config)
                .with_context(|| format!("world-model Cozo store at {}", db_path.display()))?;
        archon_cozo::run_guarded(
            "ensure world-model Cozo schema",
            ScriptMutability::Mutable,
            &config,
            || cozo_store::ensure_schema(&db),
        )?;

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
        let cozo_rows = archon_cozo::run_guarded(
            "persist world-model rows",
            ScriptMutability::Mutable,
            &self.cozo_config(),
            || cozo_store::put_rows(&self.db, rows),
        )?;

        Ok(PersistSummary {
            jsonl_rows: rows.len(),
            cozo_rows,
            ledger_path,
            db_path: self.db_path.clone(),
        })
    }

    pub fn cold_start_stats(&self) -> Result<ColdStartStats> {
        archon_cozo::run_guarded(
            "load world-model cold-start stats",
            ScriptMutability::Immutable,
            &self.cozo_config(),
            || cozo_store::cold_start_stats(&self.db),
        )
    }

    pub fn load_rows(&self) -> Result<Vec<WorldTraceRow>> {
        archon_cozo::run_guarded(
            "load world-model rows",
            ScriptMutability::Immutable,
            &self.cozo_config(),
            || cozo_store::all_rows(&self.db),
        )
    }

    pub fn row_ids(&self) -> Result<HashSet<String>> {
        archon_cozo::run_guarded(
            "load world-model row ids",
            ScriptMutability::Immutable,
            &self.cozo_config(),
            || cozo_store::row_ids(&self.db),
        )
    }

    fn cozo_config(&self) -> archon_cozo::CozoGuardConfig {
        cozo_config(&self.db_path)
    }
}

fn cozo_config(db_path: &Path) -> archon_cozo::CozoGuardConfig {
    let mut config = archon_cozo::CozoGuardConfig::for_db_path(db_path);
    config.max_attempts = COZO_LOCK_RETRY_DELAYS_MS.len() + 1;
    config.initial_backoff = std::time::Duration::from_millis(COZO_LOCK_RETRY_DELAYS_MS[0]);
    config.max_backoff =
        std::time::Duration::from_millis(*COZO_LOCK_RETRY_DELAYS_MS.last().unwrap_or(&1_600));
    config
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

    #[test]
    fn cozo_lock_classifier_matches_sqlite_lock_messages() {
        assert!(archon_cozo::is_retryable_cozo_error(
            "database is locked (code 5)"
        ));
        assert!(archon_cozo::is_retryable_cozo_error(
            "called Result::unwrap() on an Err value: database is locked"
        ));
        assert!(!archon_cozo::is_retryable_cozo_error("permission denied"));
    }

    #[test]
    fn guarded_cozo_retries_transient_lock_errors() {
        let mut attempts = 0;
        let config = archon_cozo::CozoGuardConfig {
            initial_backoff: std::time::Duration::from_millis(1),
            max_backoff: std::time::Duration::from_millis(1),
            ..Default::default()
        };

        let value = archon_cozo::run_guarded(
            "test cozo retry",
            cozo::ScriptMutability::Mutable,
            &config,
            || {
                attempts += 1;
                if attempts == 1 {
                    Err(anyhow::anyhow!("database is locked (code 5)"))
                } else {
                    Ok("ready")
                }
            },
        )
        .unwrap();

        assert_eq!(value, "ready");
        assert_eq!(attempts, 2);
    }

    #[test]
    fn guarded_cozo_retries_transient_lock_panics() {
        let mut attempts = 0;
        let config = archon_cozo::CozoGuardConfig {
            initial_backoff: std::time::Duration::from_millis(1),
            max_backoff: std::time::Duration::from_millis(1),
            ..Default::default()
        };

        let value = archon_cozo::run_guarded(
            "test cozo retry",
            cozo::ScriptMutability::Mutable,
            &config,
            || {
                attempts += 1;
                if attempts == 1 {
                    panic!("database is locked");
                }
                Ok("ready")
            },
        )
        .unwrap();

        assert_eq!(value, "ready");
        assert_eq!(attempts, 2);
    }

    #[test]
    fn guarded_cozo_does_not_retry_non_lock_errors() {
        let mut attempts = 0;
        let error = archon_cozo::run_guarded(
            "test cozo retry",
            cozo::ScriptMutability::Mutable,
            &archon_cozo::CozoGuardConfig::default(),
            || -> Result<()> {
                attempts += 1;
                Err(anyhow::anyhow!("permission denied"))
            },
        )
        .unwrap_err();

        assert!(error.to_string().contains("permission denied"));
        assert_eq!(attempts, 1);
    }

    #[test]
    fn world_model_config_uses_db_sidecar_lock() {
        let config = cozo_config(Path::new("/tmp/world-model.db"));
        assert_eq!(
            config.write_lock_path.unwrap(),
            PathBuf::from("/tmp/world-model.db.archon-cozo-write.lock")
        );
    }
}
