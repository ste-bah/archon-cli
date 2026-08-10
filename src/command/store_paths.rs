//! Shared project-local store paths for slash and CLI commands.
//!
//! Completion, evidence, docs, provenance, learning, and related views
//! need to see the same persisted facts. Keep the default in the project
//! `.archon` directory and only split stores when an explicit override is
//! provided.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::Result;
use archon_core::config::ArchonConfig;
use cozo::DbInstance;

pub(crate) const EVIDENCE_DB_ENV: &str = "ARCHON_EVIDENCE_DB_PATH";
pub(crate) const LEARNING_DB_ENV: &str = "ARCHON_LEARNING_DB_PATH";
pub(crate) const SESSION_DB_ENV: &str = "ARCHON_SESSION_DB_PATH";

/// Store-path override keys for the provenance store, most specific first.
///
/// Shared by `command::prov` and the `archon draft` provenance import so the two
/// cannot disagree about which store `archon prov trace` reads back.
pub(crate) const PROV_DB_ENV_KEYS: &[&str] = &["ARCHON_PROV_DB_PATH", "ARCHON_KB_DB_PATH"];

/// Store-path override key for the knowledge base.
pub(crate) const KB_DB_ENV_KEYS: &[&str] = &["ARCHON_KB_DB_PATH"];

#[cfg(test)]
pub(crate) static DOCS_DB_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
#[cfg(test)]
pub(crate) static LEARNING_DB_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub(crate) fn project_archon_dir_for(cwd: &Path) -> PathBuf {
    cwd.join(".archon")
}

/// Resolves a store path from `overrides` (most specific first), then the shared
/// evidence key, then the project default under `cwd`.
///
/// `lookup` supplies each key's value. Production callers pass the process
/// environment; taking it as an argument lets the precedence rule be exercised
/// with explicit inputs instead of by mutating process-global state, which in a
/// binary whose tests share one process is a data race (#166).
pub(crate) fn evidence_db_path_with(
    cwd: &Path,
    overrides: &[&str],
    lookup: impl Fn(&str) -> Option<OsString>,
) -> PathBuf {
    overrides
        .iter()
        .copied()
        .chain(std::iter::once(EVIDENCE_DB_ENV))
        .find_map(|key| lookup(key).filter(|value| !value.is_empty()))
        .map(PathBuf::from)
        .unwrap_or_else(|| project_archon_dir_for(cwd).join("archon-data.db"))
}

pub(crate) fn evidence_db_path_for_dir(cwd: &Path, overrides: &[&str]) -> PathBuf {
    // Closure rather than `std::env::var_os` directly: the generic function item is not
    // higher-ranked over the key lifetime, so it does not satisfy `Fn(&str)`.
    evidence_db_path_with(cwd, overrides, |key: &str| std::env::var_os(key))
}

pub(crate) fn evidence_db_path(overrides: &[&str]) -> PathBuf {
    evidence_db_path_for_dir(
        &std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        overrides,
    )
}

pub(crate) fn open_evidence_db(
    label: &str,
    overrides: &[&str],
) -> Result<std::sync::Arc<DbInstance>> {
    open_sqlite_db(&evidence_db_path(overrides), label)
}

pub(crate) fn learning_db_path_for_dir(cwd: &Path) -> PathBuf {
    std::env::var_os(LEARNING_DB_ENV)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| project_archon_dir_for(cwd).join("learning-state.db"))
}

pub(crate) fn learning_db_path() -> PathBuf {
    learning_db_path_for_dir(&std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

pub(crate) fn open_learning_db(label: &str) -> Result<std::sync::Arc<DbInstance>> {
    open_sqlite_db(&learning_db_path(), label)
}

pub(crate) fn session_db_path(config: &ArchonConfig) -> PathBuf {
    std::env::var_os(SESSION_DB_ENV)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| config.session.db_path.as_ref().map(PathBuf::from))
        .unwrap_or_else(archon_session::storage::default_db_path)
}

pub(crate) fn open_session_store(path: &Path) -> Result<archon_session::storage::SessionStore> {
    archon_session::storage::SessionStore::open(path)
        .map_err(|e| anyhow::anyhow!("failed to open session database at {}: {e}", path.display()))
}

pub(crate) fn open_sqlite_db(path: &Path, label: &str) -> Result<std::sync::Arc<DbInstance>> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let path_str = path.to_string_lossy().to_string();
    archon_learning::cozo_guard::open_sqlite_guarded(
        &path_str,
        &format!("open {label} store at {path_str}"),
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn sqlite_store_retains_its_write_lock_identity() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("store.db");
        let database = super::open_sqlite_db(&path, "test").unwrap();

        assert_eq!(
            archon_cozo::guarded_config_for(&database).and_then(|config| config.write_lock_path),
            Some(archon_cozo::write_lock_path_for_db(&path)),
        );
    }
}
