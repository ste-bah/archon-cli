use std::path::{Component, Path, PathBuf};

/// Use the walk's plain path spelling without treating glob syntax as a
/// filesystem name. In particular, `paths::plain` rejects wildcard components.
pub(super) fn normalize_absolute_pattern(pattern: &str) -> std::borrow::Cow<'_, str> {
    let Some(rest) = pattern.strip_prefix(r"\\?\") else {
        return pattern.into();
    };
    let literal_end = rest
        .find(['*', '?', '[', '{'])
        .map(|index| {
            rest[..index]
                .rfind(['\\', '/'])
                .map_or(0, |separator| separator + 1)
        })
        .unwrap_or(rest.len());
    let (literal, glob) = pattern.split_at(4 + literal_end);
    let plain = archon_shell::paths::plain(PathBuf::from(literal));
    format!("{}{glob}", plain.to_string_lossy()).into()
}

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
