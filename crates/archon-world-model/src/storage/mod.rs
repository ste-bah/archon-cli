//! World-model storage facade.
//!
//! The PRD calls for a split store: JSONL for append-only audit evidence and
//! Cozo for indexed joins/evaluation. This module keeps that contract small.

pub mod cozo_store;
pub mod deferred_retry;
pub mod jsonl;
pub mod retention;

use std::collections::HashSet;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
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

const COZO_LOCK_RETRY_DELAYS_MS: [u64; 7] = [25, 50, 100, 200, 400, 800, 1_600];
static COZO_PANIC_HOOK_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

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
        let db = retry_cozo_lock("open world-model Cozo store", || {
            let db = DbInstance::new("sqlite", &db_path, "")
                .map_err(|e| anyhow::anyhow!("open world-model Cozo store failed: {e}"))?;
            cozo_store::ensure_schema(&db)?;
            Ok(db)
        })
        .with_context(|| format!("world-model Cozo store at {}", db_path.display()))?;

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
        retry_cozo_lock("load world-model rows", || cozo_store::all_rows(&self.db))
    }

    pub fn row_ids(&self) -> Result<HashSet<String>> {
        cozo_store::row_ids(&self.db)
    }
}

fn retry_cozo_lock<T>(operation: &'static str, mut run: impl FnMut() -> Result<T>) -> Result<T> {
    for attempt in 0..=COZO_LOCK_RETRY_DELAYS_MS.len() {
        match catch_cozo_operation(operation, &mut run) {
            Ok(value) => return Ok(value),
            Err(error)
                if is_cozo_lock_error(&error) && attempt < COZO_LOCK_RETRY_DELAYS_MS.len() =>
            {
                let delay = COZO_LOCK_RETRY_DELAYS_MS[attempt];
                tracing::warn!(
                    operation,
                    attempt = attempt + 1,
                    delay_ms = delay,
                    error = %error,
                    "world-model Cozo store locked; retrying"
                );
                thread::sleep(Duration::from_millis(delay));
            }
            Err(error) => return Err(error),
        }
    }
    unreachable!("retry loop returns from all terminal paths")
}

fn catch_cozo_operation<T>(
    operation: &'static str,
    run: &mut impl FnMut() -> Result<T>,
) -> Result<T> {
    let hook_lock = COZO_PANIC_HOOK_LOCK.get_or_init(|| Mutex::new(()));
    let _guard = hook_lock.lock().expect("cozo panic hook lock poisoned");
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let result = catch_unwind(AssertUnwindSafe(run));
    std::panic::set_hook(hook);

    match result {
        Ok(result) => result,
        Err(payload) => Err(anyhow::anyhow!(
            "{operation} panicked: {}",
            panic_payload_message(payload)
        )),
    }
}

fn panic_payload_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_owned()
    } else {
        "unknown panic payload".to_owned()
    }
}

fn is_cozo_lock_error(error: &anyhow::Error) -> bool {
    is_cozo_lock_message(&format!("{error:#}"))
}

fn is_cozo_lock_message(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    message.contains("database is locked")
        || message.contains("database table is locked")
        || message.contains("sqlite_busy")
        || message.contains("locked (code 5)")
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
        assert!(is_cozo_lock_message("database is locked (code 5)"));
        assert!(is_cozo_lock_message(
            "called Result::unwrap() on an Err value: database is locked"
        ));
        assert!(!is_cozo_lock_message("permission denied"));
    }

    #[test]
    fn retry_cozo_lock_retries_transient_lock_errors() {
        let mut attempts = 0;

        let value = retry_cozo_lock("test cozo retry", || {
            attempts += 1;
            if attempts == 1 {
                Err(anyhow::anyhow!("database is locked (code 5)"))
            } else {
                Ok("ready")
            }
        })
        .unwrap();

        assert_eq!(value, "ready");
        assert_eq!(attempts, 2);
    }

    #[test]
    fn retry_cozo_lock_retries_transient_lock_panics() {
        let mut attempts = 0;

        let value = retry_cozo_lock("test cozo retry", || {
            attempts += 1;
            if attempts == 1 {
                panic!("database is locked");
            }
            Ok("ready")
        })
        .unwrap();

        assert_eq!(value, "ready");
        assert_eq!(attempts, 2);
    }

    #[test]
    fn retry_cozo_lock_does_not_retry_non_lock_errors() {
        let mut attempts = 0;

        let error = retry_cozo_lock("test cozo retry", || -> Result<()> {
            attempts += 1;
            Err(anyhow::anyhow!("permission denied"))
        })
        .unwrap_err();

        assert!(error.to_string().contains("permission denied"));
        assert_eq!(attempts, 1);
    }
}
