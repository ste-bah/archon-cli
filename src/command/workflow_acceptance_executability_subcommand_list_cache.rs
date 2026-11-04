//! Identify listing evidence by the resolved program and site environment.
use super::*;

pub(super) fn key(program: &Path, at: &Context) -> Option<Key> {
    let resolved = std::fs::canonicalize(program).ok()?;
    let meta = std::fs::metadata(&resolved).ok();
    let identity = (
        resolved,
        meta.as_ref().map_or(0, std::fs::Metadata::len),
        meta.and_then(|meta| meta.modified().ok()),
    );
    Some((identity, at.environment.clone()))
}
