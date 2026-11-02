//! Resolved path authority before host publication.
pub(super) fn resolved_path_is_within_root(
    root: &std::path::Path,
    path: &std::path::Path,
) -> std::io::Result<bool> {
    fn resolve(path: &std::path::Path) -> std::io::Result<std::path::PathBuf> {
        if path.exists() {
            return archon_shell::paths::canonicalize(path);
        }
        let parent = path.parent().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "path has no parent")
        })?;
        let name = path.file_name().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "path has no file name")
        })?;
        Ok(archon_shell::paths::canonicalize(parent)?.join(name))
    }

    resolved_path_is_within_root_using(root, path, resolve)
}

pub(super) fn resolved_path_is_within_root_using(
    root: &std::path::Path,
    path: &std::path::Path,
    resolve: impl Fn(&std::path::Path) -> std::io::Result<std::path::PathBuf>,
) -> std::io::Result<bool> {
    let canonical_root = resolve(root)?;
    let canonical_path = resolve(path)?;
    Ok(canonical_path.starts_with(canonical_root))
}
