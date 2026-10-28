use std::path::{Component, Path, PathBuf};

/// Only literal components before the first pattern can restrict the walk.
/// Parent steps end the prefix because their target depends on earlier paths.
pub(super) fn fixed_prefix(pattern: &Path) -> PathBuf {
    let mut prefix = PathBuf::new();
    for component in pattern.components() {
        match component {
            Component::Normal(value) if !super::is_literal_component(value) => break,
            Component::ParentDir => break,
            _ => prefix.push(component.as_os_str()),
        }
    }
    prefix
}
