//! The shell scripts sent to a remote sandbox world, and the quoting that
//! keeps a filename from becoming a command.
//!
//! Split from `remote_fs.rs` so that what Archon *asks* the far side to do
//! stays readable on its own, separately from how the answers are interpreted.
//! Every function here is pure, which is what makes the whole command layer
//! testable with no host on the other end.

use std::io;

/// Exit status the scripts use to say "the far side has no `base64`".
pub(crate) const EXIT_NO_BASE64: i32 = 97;
/// Exit status the scripts use to say "the far side's bash has no `globstar`".
pub(crate) const EXIT_NO_GLOBSTAR: i32 = 96;

const PIPEFAIL: &str = "set -o pipefail\n";
const REQUIRE_BASE64: &str = "command -v base64 >/dev/null 2>&1 || { printf 'archon-fs: base64 is not available in the sandbox world\\n' >&2; exit 97; }\n";

/// Single-quote one value for a POSIX shell.
///
/// The only escape a single-quoted string needs: end the quote, emit an
/// escaped quote, reopen. Everything else — spaces, `$`, backticks, newlines,
/// semicolons — is literal inside the quotes, which is exactly the property
/// that keeps a filename from becoming a command.
pub(crate) fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r#"'\''"#))
}

pub(crate) fn read_script(path: &str) -> String {
    format!("{PIPEFAIL}{REQUIRE_BASE64}base64 < {}\n", shell_quote(path))
}

/// Write via a sibling temp file, then rename.
///
/// A transport that dies mid-stream would otherwise leave the real file
/// truncated. The trailing byte count is not decoration: it is the only
/// evidence available that stdin actually arrived on the far side.
pub(crate) fn write_script(path: &str) -> String {
    let quoted = shell_quote(path);
    format!(
        "{PIPEFAIL}{REQUIRE_BASE64}\
if printf '' | base64 -d >/dev/null 2>&1; then decode='base64 -d'; else decode='base64 -D'; fi\n\
tmp={quoted}\".archon-fs.$$.tmp\"\n\
if ! $decode > \"$tmp\"; then status=$?; rm -f -- \"$tmp\"; exit $status; fi\n\
if ! mv -f -- \"$tmp\" {quoted}; then status=$?; rm -f -- \"$tmp\"; exit $status; fi\n\
wc -c < {quoted} | tr -d ' \\t'\n"
    )
}

pub(crate) fn create_dir_all_script(path: &str) -> String {
    format!("mkdir -p -- {}\n", shell_quote(path))
}

/// GNU `stat` first, BSD/macOS `stat` second.
///
/// `%F`/`%HT` are asked for rather than inferred from a `-d` test so that one
/// round trip answers size, mtime and directory-ness together.
pub(crate) fn metadata_script(path: &str) -> String {
    let quoted = shell_quote(path);
    format!("stat -c '%s %Y %F' -- {quoted} 2>/dev/null || stat -f '%z %m %HT' {quoted}\n")
}

/// `-print0` because a newline is a legal character in a filename.
pub(crate) fn read_dir_script(path: &str) -> String {
    format!(
        "{PIPEFAIL}{REQUIRE_BASE64}find {} -mindepth 1 -maxdepth 1 -print0 | base64\n",
        shell_quote(path)
    )
}

/// Plain `rm`, not `rm -f`: a missing file is an error here, as it is for
/// `std::fs::remove_file`, and `-f` would turn it into a silent success.
pub(crate) fn remove_file_script(path: &str) -> String {
    format!("rm -- {}\n", shell_quote(path))
}

pub(crate) fn rename_script(from: &str, to: &str) -> String {
    format!("mv -f -- {} {}\n", shell_quote(from), shell_quote(to))
}

/// The one script that interpolates something unquoted.
///
/// The pattern *must* reach the shell unquoted or it will not expand, so it is
/// admitted through [`validate_glob_pattern`] first and nowhere else.
pub(crate) fn glob_script(base: &str, pattern: &str) -> String {
    let mut script = String::from(PIPEFAIL);
    script.push_str(REQUIRE_BASE64);
    if pattern.contains("**") {
        script.push_str(
            "shopt -s globstar 2>/dev/null || { printf 'archon-fs: the bash in the sandbox world is too old for globstar, so ** cannot be matched\\n' >&2; exit 96; }\n",
        );
    }
    script.push_str("shopt -s nullglob\n");
    script.push_str(&format!("cd -- {} || exit 1\n", shell_quote(base)));
    script.push_str(&format!(
        "for entry in {pattern}; do printf '%s\\0' \"$entry\"; done | base64\n"
    ));
    script
}

/// Characters a glob pattern may contain, given that it is interpolated raw.
///
/// Everything that could end the word and start a command — quotes, `$`,
/// backtick, `;`, `&`, `|`, redirection, whitespace, backslash — is absent,
/// and so is `~`, whose expansion would silently retarget the match.
fn glob_char_is_allowed(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || "*?[]{},-_./+@!^=:%".contains(ch)
}

pub(crate) fn validate_glob_pattern(pattern: &str) -> io::Result<()> {
    if pattern.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "an empty glob pattern matches nothing in the sandbox world",
        ));
    }
    if let Some(bad) = pattern.chars().find(|ch| !glob_char_is_allowed(*ch)) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "glob pattern {pattern:?} contains {bad:?}, which the sandbox world's shell would \
                 interpret rather than match"
            ),
        ));
    }
    Ok(())
}
