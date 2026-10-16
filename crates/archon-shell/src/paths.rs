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
//! Every crate that exchanges workflow paths canonicalizes through
//! [`canonicalize`], whose [`try_plain`] step removes the verbatim prefix only
//! when a valid drive or UNC spelling under MAX_PATH names the same path.
//! [`plain`] keeps its legacy best-effort behavior for callers that accept a
//! verbatim result; Git callers must use [`try_plain`] and surface its error.
//! Off Windows `canonicalize` never adds the prefix. Rust's filesystem calls
//! add the long-path prefix back internally when a plain path needs it.
use std::path::{Path, PathBuf};

/// Best-effort plain spelling; returns the original path when its verbatim
/// prefix cannot safely be removed. Use [`try_plain`] when the caller cannot
/// consume a verbatim path.
pub fn plain(path: PathBuf) -> PathBuf {
    try_plain(path.clone()).unwrap_or(path)
}

/// Fallible plain spelling for paths sent to tools that cannot parse Windows
/// verbatim paths (notably Git).
pub fn try_plain(path: PathBuf) -> std::io::Result<PathBuf> {
    let Some(text) = path.to_str() else {
        return Ok(path);
    };
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        let mut unc_parts = rest.split(['\\', '/']);
        if unc_parts.next().is_none_or(str::is_empty) || unc_parts.next().is_none_or(str::is_empty)
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "cannot remove verbatim prefix from {}: UNC path has no server/share pair",
                    path.display()
                ),
            ));
        }
        return checked_verbatim(path.clone(), rest.to_owned(), true);
    }
    match text.strip_prefix(r"\\?\") {
        // Only a drive path (`C:\...`): a verbatim device or volume path
        // (`\\?\Volume{...}`) has no plain spelling.
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => {
            if rest.as_bytes().get(2) != Some(&b'\\') {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!(
                        "cannot remove verbatim prefix from {}: drive path is not rooted with a backslash",
                        path.display()
                    ),
                ));
            }
            checked_verbatim(path.clone(), rest.to_owned(), false)
        }
        _ => Ok(path),
    }
}

fn checked_verbatim(path: PathBuf, rest: String, unc: bool) -> std::io::Result<PathBuf> {
    let plain = if unc {
        format!(r"\\{rest}")
    } else {
        rest.clone()
    };
    let components = rest.split(['\\', '/']).skip(if unc { 0 } else { 1 });
    let reserved = ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"];
    let mut reason = None;
    for component in components {
        if matches!(component, "." | "..") {
            reason = Some(format!(
                "component {component:?} is normalized by a plain path"
            ));
            break;
        }
        if let Some(character) = component
            .chars()
            .find(|character| *character < ' ' || "<>:\"|?*".contains(*character))
        {
            reason = Some(format!(
                "component {component:?} contains invalid character {character:?}"
            ));
            break;
        }
        if component.ends_with(['.', ' ']) {
            reason = Some(format!("component {component:?} ends in a dot or space"));
            break;
        }
        let stem = component
            .split('.')
            .next()
            .unwrap_or(component)
            .trim_end_matches(['.', ' ']);
        let upper = stem.to_ascii_uppercase();
        if reserved.contains(&upper.as_str())
            || ["COM", "LPT"].iter().any(|prefix| {
                upper.strip_prefix(prefix).is_some_and(|suffix| {
                    matches!(
                        suffix,
                        "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                    )
                })
            })
        {
            reason = Some(format!(
                "component {component:?} is a reserved Windows device name"
            ));
            break;
        }
    }
    if reason.is_none() && plain.encode_utf16().count() >= 260 {
        reason = Some(format!(
            "plain spelling is {} UTF-16 code units (must be under 260)",
            plain.encode_utf16().count()
        ));
    }
    if let Some(reason) = reason {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "cannot remove verbatim prefix from {}: {reason}",
                path.display()
            ),
        ));
    }
    Ok(PathBuf::from(plain))
}

/// `std::fs::canonicalize`, in its one safe plain spelling ([`try_plain`]).
pub fn canonicalize(path: impl AsRef<Path>) -> std::io::Result<PathBuf> {
    try_plain(std::fs::canonicalize(path)?)
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

    #[test]
    fn verbatim_paths_that_change_meaning_are_rejected_with_path_and_reason() {
        for (given, reason) in [
            (r"\\?\C:\work\name.\file", "ends in a dot or space"),
            (r"\\?\C:\work\name \file", "ends in a dot or space"),
            (r"\\?\C:\work\CON\file", "reserved Windows device name"),
            (
                r"\\?\UNC\server\share\NUL.txt",
                "reserved Windows device name",
            ),
            (r"\\?\C:\work\folder\..\file", "normalized by a plain path"),
            (r"\\?\UNC\server\share\bad?name", "invalid character"),
            (&format!(r"\\?\C:\{}", "a".repeat(260)), "under 260"),
        ] {
            let error = try_plain(PathBuf::from(given)).unwrap_err().to_string();
            assert!(error.contains(given), "{error}");
            assert!(error.contains(reason), "{error}");
        }
    }
}
