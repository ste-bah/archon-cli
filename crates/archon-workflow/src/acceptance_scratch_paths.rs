//! Issue-234: test fixtures name temp paths inside generated shell commands.
//! Issue-438: stored and compared relative paths use '/' on every OS.
use std::path::{Component, Path};

/// A relative path as a map key or digest line: its components joined with
/// '/', whatever the platform separator. On Unix this is the path's own text;
/// on Windows `tasks\set\config.json` becomes `tasks/set/config.json`, so
/// audit snapshots, inventories and digests name a path identically on every
/// OS. A component's own bytes are never rewritten.
pub fn relative_key(path: &Path) -> String {
    path.components()
        .map(|component| match component {
            Component::Normal(part) => part.to_string_lossy(),
            other => other.as_os_str().to_string_lossy(),
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Whether `path` is a strict component-wise descendant of `ancestor`.
/// Component comparisons are case-sensitive; case-only nesting on APFS or
/// Windows remains outside this helper's guarantee.
pub fn is_strict_descendant(path: &Path, ancestor: &Path) -> bool {
    let mut path_components = path.components();
    let mut ancestor_components = ancestor.components();
    let mut matched = false;
    loop {
        match (path_components.next(), ancestor_components.next()) {
            (_, Some(Component::ParentDir)) | (Some(Component::ParentDir), _) => return false,
            (Some(path), Some(ancestor)) if path == ancestor => matched = true,
            (Some(_), Some(_)) | (None, Some(_)) => return false,
            (Some(_), None) => return matched,
            (None, None) => return false,
        }
    }
}

/// A path as a single POSIX shell argument: the Windows verbatim prefix
/// removed, backslashes turned to forward slashes, single-quoted. A Windows
/// path interpolated into a shell command raw loses its backslashes to the
/// shell and names a bogus relative file.
#[cfg(test)]
pub(crate) fn shell_arg(path: &Path) -> String {
    let plain = archon_shell::paths::plain(path.to_path_buf());
    format!(
        "'{}'",
        plain
            .to_string_lossy()
            .replace('\\', "/")
            .replace('\'', r"'\''")
    )
}

#[cfg(test)]
mod tests {
    use super::relative_key;
    use std::path::{Path, PathBuf};

    #[test]
    fn relative_key_joins_components_with_forward_slashes() {
        // Built from components, so it renders with the platform separator.
        let path: PathBuf = ["tasks", "set", "config.json"].iter().collect();
        assert_eq!(relative_key(&path), "tasks/set/config.json");
        let joined = Path::new("tasks").join("set").join(".git").join("config");
        assert_eq!(relative_key(&joined), "tasks/set/.git/config");
        assert_eq!(relative_key(Path::new("config.json")), "config.json");
        assert_eq!(relative_key(Path::new("")), "");
        #[cfg(windows)]
        assert_eq!(
            relative_key(Path::new(r"tasks\set\config.json")),
            "tasks/set/config.json"
        );
    }
}
