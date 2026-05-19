//! Shared project-local store paths for slash and CLI commands.
//!
//! Completion, evidence, docs, provenance, learning, and related views
//! need to see the same persisted facts. Keep the default in the project
//! `.archon` directory and only split stores when an explicit override is
//! provided.

use std::path::{Path, PathBuf};

use anyhow::Result;
use archon_core::config::ArchonConfig;
use cozo::DbInstance;

pub(crate) const EVIDENCE_DB_ENV: &str = "ARCHON_EVIDENCE_DB_PATH";
pub(crate) const SESSION_DB_ENV: &str = "ARCHON_SESSION_DB_PATH";

pub(crate) fn project_archon_dir_for(cwd: &Path) -> PathBuf {
    cwd.join(".archon")
}

pub(crate) fn evidence_db_path_for_dir(cwd: &Path, overrides: &[&str]) -> PathBuf {
    overrides
        .iter()
        .copied()
        .chain(std::iter::once(EVIDENCE_DB_ENV))
        .find_map(|key| std::env::var_os(key).filter(|value| !value.is_empty()))
        .map(PathBuf::from)
        .unwrap_or_else(|| project_archon_dir_for(cwd).join("archon-data.db"))
}

pub(crate) fn evidence_db_path(overrides: &[&str]) -> PathBuf {
    evidence_db_path_for_dir(
        &std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        overrides,
    )
}

pub(crate) fn open_evidence_db(label: &str, overrides: &[&str]) -> Result<DbInstance> {
    open_sqlite_db(&evidence_db_path(overrides), label)
}

pub(crate) fn session_db_path(config: &ArchonConfig) -> PathBuf {
    std::env::var_os(SESSION_DB_ENV)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            config
                .session
                .db_path
                .as_ref()
                .map(|path| PathBuf::from(path))
        })
        .unwrap_or_else(archon_session::storage::default_db_path)
}

pub(crate) fn open_session_store(path: &Path) -> Result<archon_session::storage::SessionStore> {
    archon_session::storage::SessionStore::open(path)
        .map_err(|e| anyhow::anyhow!("failed to open session database at {}: {e}", path.display()))
}

pub(crate) fn open_sqlite_db(path: &Path, label: &str) -> Result<DbInstance> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let path_str = path.to_string_lossy().to_string();
    DbInstance::new("sqlite", &path_str, "")
        .map_err(|e| anyhow::anyhow!("failed to open {label} store at {path_str}: {e}"))
}
