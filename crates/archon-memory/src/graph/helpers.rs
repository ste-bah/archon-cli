use std::collections::BTreeMap;

use cozo::{DataValue, DbInstance, NamedRows, ScriptMutability};

use crate::types::MemoryError;

/// Run a `Mutable` Cozo script under the shared `archon-cozo` write guard.
///
/// The memory graph is SQLite-backed and therefore single-writer; routing every
/// mutation through the guard gives it the process mutex, the cross-process file
/// lock, and the SQLITE_BUSY retry loop instead of failing hard on a busy store.
pub(crate) fn run_mutable(
    db: &DbInstance,
    script: &str,
    params: BTreeMap<String, DataValue>,
    context: &str,
) -> Result<NamedRows, MemoryError> {
    archon_cozo::run_bound_script_guarded(db, script, params, ScriptMutability::Mutable, context)
        .map_err(db_err)
}

/// Helper to convert CozoDB errors into MemoryError.
pub(super) fn db_err(e: impl std::fmt::Display) -> MemoryError {
    MemoryError::Database(e.to_string())
}

#[cfg(unix)]
pub(super) fn secure_file_permissions(path: &std::path::Path) -> Result<(), std::io::Error> {
    use std::os::unix::fs::PermissionsExt;
    let perms = std::fs::Permissions::from_mode(0o600);
    std::fs::set_permissions(path, perms)
}

pub(super) fn empty_rows() -> NamedRows {
    NamedRows::new(vec![], vec![])
}
