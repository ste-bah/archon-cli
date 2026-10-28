use std::path::{Path, PathBuf};

pub fn diagnostic_path(
    token: &str,
    mapped_sibling: Option<String>,
    manifest_sibling: Option<String>,
    paths: (&Path, &Path),
) -> String {
    if matches!(token, "." | "..") {
        let mapped = relative_path(paths.0, &normalize_absolute(paths.1));
        if mapped.as_os_str().is_empty() {
            ".".into()
        } else {
            mapped.to_string_lossy().into_owned()
        }
    } else {
        mapped_sibling.or(manifest_sibling).unwrap_or_else(|| {
            relative_path(paths.0, paths.1)
                .to_string_lossy()
                .into_owned()
        })
    }
}

pub fn normalize_absolute(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

pub fn relative_path(from: &Path, to: &Path) -> PathBuf {
    let from: Vec<_> = from.components().collect();
    let to: Vec<_> = to.components().collect();
    let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    let mut relative = PathBuf::new();
    for _ in common..from.len() {
        relative.push("..");
    }
    for component in &to[common..] {
        relative.push(component.as_os_str());
    }
    relative
}

/// A relative spelling of the selected tree or one of its ancestors.
pub fn whole_tree_path(text: &str) -> bool {
    Path::new(text).components().all(|component| {
        matches!(
            component,
            std::path::Component::CurDir | std::path::Component::ParentDir
        )
    })
}

/// Existing targets are classified by identity, including symlink aliases.
/// Keep a lexical fallback only when the target cannot be resolved.
pub fn whole_tree_target(text: &str, root: &Path, resolved: Option<&Path>) -> bool {
    resolved.map_or_else(|| whole_tree_path(text), |target| root.starts_with(target))
}

pub fn normalize(token: &str) -> &str {
    let normalized = token.trim_start_matches("./").trim_end_matches('/');
    if normalized.is_empty() && !token.is_empty() {
        "."
    } else {
        normalized
    }
}
