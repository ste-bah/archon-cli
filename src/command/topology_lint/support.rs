use std::path::{Path, PathBuf};

pub(super) fn read_json<T: for<'de> serde::Deserialize<'de>>(
    path: &Path,
    label: &str,
    freeze: &str,
) -> Result<T, String> {
    let bytes = std::fs::read(path).map_err(|error| {
        format!(
            "required {label} {} could not be read: {error}; run workflow {freeze}",
            shown(path)
        )
    })?;
    serde_json::from_slice(&bytes).map_err(|error| {
        format!(
            "{label} {} is malformed: {error}; re-run workflow {freeze}",
            shown(path)
        )
    })
}

pub(super) fn absolute(cwd: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}

/// A path as the lint report stores it: `/`-separated (Issue 438).
pub(super) fn shown(path: &Path) -> String {
    archon_shell::paths::stored_text(path)
}
