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

#[cfg(test)]
mod tests {
    use super::*;

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
