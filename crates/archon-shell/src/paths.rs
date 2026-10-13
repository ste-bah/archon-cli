//! Issue-234: one spelling for a canonical path on every platform.
//!
//! On Windows `std::fs::canonicalize` returns a verbatim path (`\\?\C:\...`).
//! The workflow layer compares canonical paths with `starts_with`, joins
//! relative declarations onto them, hands them to git and to a POSIX shell, and
//! writes them into records other code reads back. The verbatim form breaks
//! each of those: git cannot create a worktree under it, MSYS bash cannot open
//! it, `Path::join` onto it silently normalizes `..` away (so a climbing
//! declaration is no longer seen to climb), `components()` parses it with a
//! different prefix than its plain twin, and a forward-slashed copy of it
//! (`//?/C:/...`) is parsed as a UNC share. Mixing the two spellings made the
//! same directory compare unequal to itself.
//!
//! Every crate that exchanges workflow paths therefore canonicalizes through
//! [`plain`]: the verbatim prefix is removed when the path is representable
//! without it (a drive or UNC path), so one directory has one canonical
//! spelling everywhere. Off Windows `canonicalize` never adds the prefix and
//! [`plain`] returns its input unchanged. Rust's own filesystem calls add the
//! long-path prefix back internally when a plain path needs it.
use std::path::{Path, PathBuf};

/// `path` without a Windows verbatim (`\\?\C:\...`) or verbatim-UNC
/// (`\\?\UNC\server\share\...`) prefix; any other path unchanged.
pub fn plain(path: PathBuf) -> PathBuf {
    let Some(text) = path.to_str() else {
        return path;
    };
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    match text.strip_prefix(r"\\?\") {
        // Only a drive path (`C:\...`): a verbatim device or volume path
        // (`\\?\Volume{...}`) has no plain spelling.
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => PathBuf::from(rest),
        _ => path,
    }
}

/// `std::fs::canonicalize`, in its one plain spelling ([`plain`]).
pub fn canonicalize(path: impl AsRef<Path>) -> std::io::Result<PathBuf> {
    std::fs::canonicalize(path).map(plain)
}

/// `path` as a stored record spells it (Issue 438): its [`plain`] spelling
/// with `/` between components, on a host whose separator is `\`. Records
/// are then byte-comparable across platforms, and Windows reads the text back
/// as the same path (`C:/a/b` and `//server/share/x` parse as their `\`
/// twins). A verbatim device or volume path (`\\?\Volume{...}`) has no `/`
/// spelling and is kept as it is. On a host whose separator is `/` the text
/// is the path's own: `\` is an ordinary name character there.
pub fn stored_text(path: &Path) -> String {
    stored_text_for(&path.to_string_lossy(), std::path::MAIN_SEPARATOR == '\\')
}

/// [`stored_text`] for `text`, given whether the host separator is `\`.
fn stored_text_for(text: &str, backslash_separator: bool) -> String {
    if !backslash_separator {
        return text.to_string();
    }
    let plain = plain(PathBuf::from(text));
    let plain = plain.to_string_lossy();
    if plain.starts_with(r"\\?\") || plain.starts_with(r"\\.\") {
        return plain.into_owned();
    }
    plain.replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_windows_stored_path_uses_forward_slashes_and_keeps_its_prefix() {
        for (given, expected) in [
            (
                r"C:\Users\runner\project\tasks\PRD-X",
                "C:/Users/runner/project/tasks/PRD-X",
            ),
            (r"\\?\C:\Users\a\project", "C:/Users/a/project"),
            (r"\\?\UNC\server\share\x", "//server/share/x"),
            (r"\\server\share\x", "//server/share/x"),
            (r"\\?\Volume{1234}\x", r"\\?\Volume{1234}\x"),
            (r"\\.\pipe\name", r"\\.\pipe\name"),
            (r"C:/mixed\separators", "C:/mixed/separators"),
            ("C:", "C:"),
            ("", ""),
            (r"tasks\PRD-X\TASK-1.md", "tasks/PRD-X/TASK-1.md"),
        ] {
            assert_eq!(stored_text_for(given, true), expected, "{given}");
        }
    }

    #[test]
    fn a_posix_stored_path_is_its_own_text_backslashes_included() {
        for given in ["/tmp/project/tasks", r"/tmp/odd\name", r"\\?\C:\x", ""] {
            assert_eq!(stored_text_for(given, false), given);
        }
        #[cfg(unix)]
        assert_eq!(stored_text(Path::new(r"/tmp/odd\name")), r"/tmp/odd\name");
    }

    /// A pin written before Issue 438 (`\`) and one written after (`/`)
    /// name the same directory when read back on Windows.
    #[cfg(windows)]
    #[test]
    fn windows_reads_a_stored_path_back_as_the_same_path() {
        let dir = std::env::temp_dir();
        let canonical = canonicalize(&dir).unwrap();
        let stored = stored_text(&canonical);
        assert!(!stored.contains('\\'), "{stored}");
        assert_eq!(PathBuf::from(&stored), canonical);
        assert_eq!(canonicalize(PathBuf::from(&stored)).unwrap(), canonical);
        let legacy = canonical.display().to_string();
        assert_eq!(PathBuf::from(&legacy), PathBuf::from(&stored));
    }

    #[test]
    fn a_verbatim_drive_or_unc_path_loses_its_prefix_and_nothing_else_changes() {
        for (given, expected) in [
            (r"\\?\C:\Users\a\project", r"C:\Users\a\project"),
            (r"\\?\UNC\server\share\x", r"\\server\share\x"),
            (r"\\?\Volume{1234}\x", r"\\?\Volume{1234}\x"),
            (r"C:\plain", r"C:\plain"),
            ("/usr/local/bin", "/usr/local/bin"),
            ("relative/path", "relative/path"),
        ] {
            assert_eq!(plain(PathBuf::from(given)), PathBuf::from(expected));
        }
    }
}
