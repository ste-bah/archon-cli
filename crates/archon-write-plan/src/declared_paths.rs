//! Shared extraction and normalization for declared write paths.

use std::path::{Component, Path};

/// The text of an unordered Markdown list item, with surrounding whitespace
/// removed. Shared by task readers so their bullet grammars cannot drift.
pub fn markdown_bullet_item(trimmed: &str) -> Option<&str> {
    ["- ", "* ", "+ "]
        .iter()
        .find_map(|marker| trimmed.strip_prefix(marker))
        .map(str::trim)
}

/// Return paths declared by one Files Expected to Change bullet.
///
/// An opening path token is the subject and is the only declaration read from
/// that bullet. A short prose label ending in `:` may introduce multiple
/// backticked path spans. Other prose is not a declaration.
pub fn declared_file_paths(bullet: &str) -> Vec<String> {
    let bullet = strip_list_marker(bullet);
    if let Some(subject) = opening_path(bullet) {
        return vec![subject];
    }

    let Some((label, rest)) = bullet.split_once(':') else {
        return Vec::new();
    };
    let label_words = label.split_whitespace().count();
    if label_words == 0 || label_words > 6 {
        return Vec::new();
    }
    backtick_spans(rest)
        .map(|span| span.replace('\\', "/"))
        .filter(|span| looks_like_path(span))
        .collect()
}

/// Return the opening subject path, preserving the legacy single-subject API.
pub fn declared_file_subject(bullet: &str) -> Option<String> {
    opening_path(strip_list_marker(bullet))
}

fn strip_list_marker(bullet: &str) -> &str {
    let bullet = bullet.trim();
    bullet
        .strip_prefix('-')
        .or_else(|| bullet.strip_prefix('*'))
        .or_else(|| bullet.strip_prefix('+'))
        .unwrap_or(bullet)
        .trim_start()
}

fn opening_path(bullet: &str) -> Option<String> {
    let candidate = if let Some(tail) = bullet.strip_prefix('`') {
        tail.split_once('`').map(|(subject, _)| subject)?
    } else {
        bullet.split_whitespace().next()?
    };
    if candidate.contains(char::is_whitespace) {
        return None;
    }
    let candidate = strip_trailing_sentence_punctuation(candidate).replace('\\', "/");
    if candidate.is_empty() || !looks_like_path(&candidate) {
        return None;
    }
    Some(candidate)
}

fn backtick_spans(text: &str) -> impl Iterator<Item = &str> {
    text.split('`').skip(1).step_by(2)
}

/// Normalize a declared path using the same lexical rules as forbidden entries.
/// Absolute paths are accepted only when they are inside `repository_root`.
pub fn normalize_declared_path(candidate: &str, repository_root: &Path) -> Option<String> {
    let candidate = candidate.trim_matches(['`', '"', '\'']);
    if candidate.is_empty() || candidate.contains(char::is_whitespace) {
        return None;
    }
    let candidate = candidate.replace('\\', "/");
    let path = Path::new(&candidate);
    let candidate = if path.is_absolute() {
        path.strip_prefix(repository_root).ok()?
    } else {
        path
    };
    let mut components = Vec::new();
    for component in candidate.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(part) => components.push(part.to_string_lossy().into_owned()),
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    let normalized = components.join("/");
    (!normalized.is_empty()).then_some(normalized)
}

fn strip_trailing_sentence_punctuation(candidate: &str) -> &str {
    candidate.trim_end_matches([',', ';', ':', ')'])
}

fn looks_like_path(path: &str) -> bool {
    if path.is_empty() || path.contains(char::is_whitespace) {
        return false;
    }
    if path.contains('/') {
        return true;
    }
    path.rsplit_once('.').is_some_and(|(stem, extension)| {
        !stem.is_empty()
            && (1..=4).contains(&extension.len())
            && extension
                .chars()
                .all(|character| character.is_ascii_alphanumeric())
    })
}

#[cfg(test)]
mod tests {
    use super::{declared_file_subject, normalize_declared_path};
    use std::path::Path;

    #[test]
    fn reads_only_the_opening_subject_path() {
        assert_eq!(
            declared_file_subject("- `src/a.rs` — updates `state.load` helper"),
            Some("src/a.rs".into())
        );
        assert_eq!(
            declared_file_subject("- src/a.rs — exists"),
            Some("src/a.rs".into())
        );
        assert_eq!(
            declared_file_subject("- Likely anchors: `src/a.rs`, `src/b.rs`"),
            None
        );
        assert_eq!(
            super::declared_file_paths("- Likely anchors: `src/a.rs`, `src/b.rs`, relevant files"),
            ["src/a.rs", "src/b.rs"]
        );
        assert!(
            super::declared_file_paths("- `state_load` is mentioned before `src/a.rs`").is_empty()
        );
        assert_eq!(
            super::declared_file_paths("- `src/a.rs` and `src/b.rs`"),
            ["src/a.rs"]
        );
        assert_eq!(
            declared_file_subject("- /repo/src/a.rs — exists"),
            Some("/repo/src/a.rs".into())
        );
        assert_eq!(
            declared_file_subject("- `.mcp.json` — exists"),
            Some(".mcp.json".into())
        );
        assert_eq!(
            normalize_declared_path("/repo/src/a.rs", Path::new("/repo")),
            Some("src/a.rs".into())
        );
        assert_eq!(
            normalize_declared_path(".mcp.json", Path::new("/repo")),
            Some(".mcp.json".into())
        );
    }
}
