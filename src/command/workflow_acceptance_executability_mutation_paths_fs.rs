use std::path::{Path, PathBuf};

pub fn diagnostic_path(
    token: &str,
    mapped_sibling: Option<String>,
    manifest_sibling: Option<String>,
    paths: (&Path, &Path),
) -> String {
    if matches!(token, "." | "..") {
        token.to_string()
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
