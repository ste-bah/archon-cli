use cozo::NamedRows;

use crate::types::MemoryError;

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
