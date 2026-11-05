use std::path::{Component, Path, PathBuf};

pub(super) fn normalize_root_join(project_root: &Path, path: &Path) -> PathBuf {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        project_root.join(path)
    };
    normalize_path(&joined)
}

pub(super) fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    normalized.push(component.as_os_str());
                }
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

/// Project-relative path for messages and gate output, always `/`-separated.
///
/// `Path::display` emits native separators, so the same repository produced
/// `src/foo.rs` on Unix and `src\foo.rs` on Windows. These strings are compared
/// against declared candidate paths and surfaced in gate reasons, so the
/// platform leaked into both behaviour and output.
pub(super) fn display_path(project_root: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(project_root).unwrap_or(path);
    relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

pub(super) fn display_debug_path(path: &Path) -> String {
    format!("{path:?}")
}
