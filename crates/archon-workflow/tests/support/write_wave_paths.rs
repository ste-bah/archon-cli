//! Issue-234: platform-correct paths for the write-wave fixtures. Windows
//! absolute paths carry a drive letter and a `\\?\` canonical prefix, which the
//! Unix-shaped fixture literals and raw interpolations do not expect.
use std::path::{Path, PathBuf};

/// A platform-valid `toolchain_path` for a fixture policy. `ScratchPolicy`
/// validates that every PATH entry is absolute; `/usr/bin:/bin` is a single
/// non-absolute entry on Windows (where `split_paths` splits on `;`), which
/// makes the whole policy invalid and drops its recorded project inputs.
pub fn toolchain_path() -> String {
    if cfg!(windows) {
        std::env::join_paths([archon_shell::resolve_posix_shell().parent().unwrap()])
            .unwrap()
            .into_string()
            .unwrap()
    } else {
        "/usr/bin:/bin".to_string()
    }
}

/// A path as a POSIX shell sees it: the Windows `\\?\` verbatim prefix removed,
/// backslashes turned to forward slashes, single-quoted. A path interpolated
/// into a shell command raw loses its backslashes to the shell and writes a
/// bogus relative file instead.
pub fn shell_path(path: &Path) -> String {
    let text = path.to_string_lossy();
    let text = text
        .strip_prefix(r"\\?\UNC\")
        .map(|rest| format!(r"\\{rest}"))
        .or_else(|| text.strip_prefix(r"\\?\").map(str::to_string))
        .unwrap_or_else(|| text.to_string());
    format!("'{}'", text.replace('\\', "/").replace('\'', r"'\''"))
}

/// `path` rebuilt with the platform separator: on Windows `join("a/b")` keeps
/// the `/` as given, so the spelling differs from the canonical one the host
/// records for the same file. A no-op elsewhere.
pub fn native(path: &Path) -> PathBuf {
    path.components().collect()
}

/// Whether `text` contains `needle`, with `/` and `\\` taken as one
/// separator: prompt text may spell a Windows path either way.
pub fn contains_path_text(text: &str, needle: &str) -> bool {
    text.replace('\\', "/").contains(&needle.replace('\\', "/"))
}
