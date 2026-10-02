//! Issue-234: test fixtures name temp paths inside generated shell commands.
use std::path::Path;

/// A path as a single POSIX shell argument: the Windows verbatim prefix
/// removed, backslashes turned to forward slashes, single-quoted. A Windows
/// path interpolated into a shell command raw loses its backslashes to the
/// shell and names a bogus relative file.
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
