use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use cozo::{DataValue, DbInstance, NamedRows, ScriptMutability};

pub struct SessionDb {
    inner: DbInstance,
    #[cfg(test)]
    query_count: std::sync::atomic::AtomicUsize,
}

impl SessionDb {
    fn new(inner: DbInstance) -> Self {
        Self {
            inner,
            #[cfg(test)]
            query_count: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    pub fn run_script(
        &self,
        script: &str,
        params: BTreeMap<String, DataValue>,
        mutability: ScriptMutability,
    ) -> Result<NamedRows, cozo::Error> {
        #[cfg(test)]
        self.query_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.inner.run_script(script, params, mutability)
    }
}

impl std::ops::Deref for SessionDb {
    type Target = DbInstance;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("session database error: {0}")]
    DbError(String),
    #[error("session I/O error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("session not found: {0}")]
    NotFound(String),
    #[error("refusing to replace session messages with an empty list")]
    EmptyReplaceRefused,
    #[error("message index {index} would skip current logical count {message_count}")]
    MessageIndexGap { index: u64, message_count: u64 },
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SessionMetadata {
    pub id: String,
    pub created_at: String,
    pub last_active: String,
    pub working_directory: String,
    pub git_branch: Option<String>,
    pub model: String,
    pub message_count: u64,
    pub total_tokens: u64,
    pub total_cost: f64,
    pub schema_version: u32,
    pub name: Option<String>,
    pub parent_session_id: Option<String>,
}

pub(crate) fn db_err(error: impl std::fmt::Display) -> SessionError {
    SessionError::DbError(error.to_string())
}

pub(crate) fn extract_str(value: &DataValue) -> String {
    value.get_str().unwrap_or("").to_string()
}

pub(crate) fn extract_i64(value: &DataValue) -> i64 {
    value.get_int().unwrap_or(0)
}

pub(crate) fn extract_f64(value: &DataValue) -> f64 {
    value.get_float().unwrap_or(0.0)
}

#[cfg(unix)]
fn secure_file_permissions(path: &Path) -> Result<(), std::io::Error> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

fn empty_rows() -> NamedRows {
    NamedRows::new(vec![], vec![])
}

pub struct SessionStore {
    db: SessionDb,
    #[cfg(test)]
    fail_next_replace_after_rows: std::sync::atomic::AtomicBool,
    #[cfg(test)]
    fail_next_delete_after_compaction: std::sync::atomic::AtomicBool,
    #[cfg(test)]
    fail_next_compaction_close_after_body: std::sync::atomic::AtomicBool,
    #[cfg(test)]
    fail_next_compaction_close_after_records: std::sync::atomic::AtomicBool,
    #[cfg(test)]
    delete_before_compaction_close_transaction: std::sync::atomic::AtomicBool,
}

impl SessionStore {
    pub fn open(path: &Path) -> Result<Self, SessionError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let path_str = path.to_string_lossy().to_string();
        let db = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            DbInstance::new("sqlite", &path_str, "")
        }))
        .map_err(|_| {
            SessionError::DbError(
                "cozo panicked during sqlite init — concurrent access or filesystem error".into(),
            )
        })?
        .map_err(db_err)?;
        #[cfg(unix)]
        secure_file_permissions(path)?;
        let store = Self {
            db: SessionDb::new(db),
            #[cfg(test)]
            fail_next_replace_after_rows: std::sync::atomic::AtomicBool::new(false),
            #[cfg(test)]
            fail_next_delete_after_compaction: std::sync::atomic::AtomicBool::new(false),
            #[cfg(test)]
            fail_next_compaction_close_after_body: std::sync::atomic::AtomicBool::new(false),
            #[cfg(test)]
            fail_next_compaction_close_after_records: std::sync::atomic::AtomicBool::new(false),
            #[cfg(test)]
            delete_before_compaction_close_transaction: std::sync::atomic::AtomicBool::new(false),
        };
        store.init_schema()?;
        Ok(store)
    }

    pub fn open_default() -> Result<Self, SessionError> {
        Self::open(&default_db_path())
    }

    pub fn db(&self) -> &SessionDb {
        &self.db
    }

    fn init_schema(&self) -> Result<(), SessionError> {
        self.create_relation(
            ":create sessions {
                id: String =>
                created_at: String, last_active: String, working_directory: String,
                git_branch: String, model: String, message_count: Int, total_tokens: Int,
                total_cost: Float, schema_version: Int
            }",
        )?;
        self.create_relation(
            ":create messages {
                session_id: String, message_index: Int => content: String
            }",
        )?;
        self.create_relation(":create session_tags { session_id: String, tag: String }")?;
        self.create_relation(":create session_names { session_id: String => name: String }")?;
        self.create_relation(
            ":create session_parents { session_id: String => parent_session_id: String }",
        )?;
        self.create_relation(
            ":create compaction_segments {
                id: String => session_id: String, start_index: Int, end_index: Int,
                status: String, summary: String, model: String, attribution: String,
                failure: String, input_tokens: Int, output_tokens: Int, cost: Float,
                created_at: String, updated_at: String
            }",
        )?;
        self.create_relation(":create compaction_segment_bodies { id: String => body: String }")?;
        self.create_relation(
            ":create compaction_ledger {
                id: String => session_id: String, kind: String, payload: String,
                start_index: Int, end_index: Int, created_at: String
            }",
        )?;
        self.create_relation(
            ":create compaction_telemetry {
                id: String => session_id: String, action: String, payload: String,
                created_at: String
            }",
        )
    }

    fn create_relation(&self, script: &str) -> Result<(), SessionError> {
        self.db
            .run_script(script, BTreeMap::new(), ScriptMutability::Mutable)
            .or_else(|error| {
                let message = error.to_string();
                if message.contains("already exists") || message.contains("conflicts") {
                    Ok(empty_rows())
                } else {
                    Err(db_err(error))
                }
            })?;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn fail_next_replace_after_rows_are_written(&self) {
        self.fail_next_replace_after_rows
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    #[cfg(test)]
    pub(crate) fn fail_next_delete_after_compaction_rows(&self) {
        self.fail_next_delete_after_compaction
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    #[cfg(test)]
    pub(crate) fn fail_next_compaction_close_after_body(&self) {
        self.fail_next_compaction_close_after_body
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    #[cfg(test)]
    pub(crate) fn fail_next_compaction_close_after_records(&self) {
        self.fail_next_compaction_close_after_records
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    #[cfg(test)]
    pub(crate) fn delete_before_next_compaction_close_transaction(&self) {
        self.delete_before_compaction_close_transaction
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    #[cfg(test)]
    pub(crate) fn reset_query_count(&self) {
        self.db
            .query_count
            .store(0, std::sync::atomic::Ordering::SeqCst);
    }

    #[cfg(test)]
    pub(crate) fn query_count(&self) -> usize {
        self.db
            .query_count
            .load(std::sync::atomic::Ordering::SeqCst)
    }
}

pub fn default_db_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from(".local/share"))
        .join("archon")
        .join("sessions")
        .join("sessions.db")
}

#[path = "storage_compaction.rs"]
mod storage_compaction;
#[path = "storage_compaction_close.rs"]
mod storage_compaction_close;
#[path = "storage_compaction_codec.rs"]
mod storage_compaction_codec;
#[path = "storage_compaction_lifecycle.rs"]
mod storage_compaction_lifecycle;
pub use storage_compaction::{
    CompactionLedgerRecord, CompactionSegment, CompactionSummaryStatus, CompactionTelemetryRecord,
};
#[path = "storage_listing.rs"]
mod storage_listing;
#[path = "storage_messages.rs"]
mod storage_messages;
#[path = "storage_relations.rs"]
mod storage_relations;
#[path = "storage_session_ops.rs"]
mod storage_session_ops;

#[cfg(test)]
#[path = "storage_atomicity_tests.rs"]
mod storage_atomicity_tests;
#[cfg(test)]
#[path = "storage_compaction_close_persistence_tests.rs"]
mod storage_compaction_close_persistence_tests;
#[cfg(test)]
#[path = "storage_compaction_close_tests.rs"]
mod storage_compaction_close_tests;
#[cfg(test)]
#[path = "storage_compaction_tests.rs"]
mod storage_compaction_tests;
