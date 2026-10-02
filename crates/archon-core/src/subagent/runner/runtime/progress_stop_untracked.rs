//! Fingerprinting the untracked entries of a working tree, each by what it is.
//!
//! Handing every listed path to `git hash-object` failed the whole query on
//! the first entry it could not hash — a link to a directory, a dangling link,
//! a nested repository — and that disabled the progress check for the rest of
//! the session. Here every entry is fingerprinted on its own: a regular file by
//! its contents (bounded, see [`Caps`]), a link by its target, a nested
//! repository by its `HEAD`, and anything else is skipped with a note. No
//! entry can stop the digest.

use std::io::Read;
use std::path::Path;

use super::tree::Fnv;

/// How much content the fingerprint reads in one round. Past either bound a
/// file is fingerprinted by its size and modification time instead, which
/// still moves when it changes but cannot see a revert.
#[derive(Debug, Clone, Copy)]
pub(super) struct Caps {
    /// Files whose contents are read.
    pub(super) files: usize,
    /// Bytes read across them.
    pub(super) bytes: u64,
}

impl Default for Caps {
    fn default() -> Self {
        Self {
            files: 2_000,
            bytes: 32 * 1024 * 1024,
        }
    }
}

/// One fingerprint over the NUL-separated `listed` paths (as `git ls-files -o
/// -z` prints them) relative to `root`.
pub(super) fn fingerprint(root: &Path, listed: &[u8], caps: Caps) -> u64 {
    let mut hash = Fnv::default();
    let mut files = 0usize;
    let mut bytes = 0u64;
    let mut capped = false;
    for raw in listed
        .split(|byte| *byte == 0)
        .filter(|raw| !raw.is_empty())
    {
        let name = String::from_utf8_lossy(raw);
        let path = root.join(name.trim_end_matches('/'));
        hash.feed(raw);
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            tracing::debug!(path = %path.display(), "untracked entry vanished; fingerprinted by name");
            continue;
        };
        let kind = meta.file_type();
        if kind.is_symlink() {
            let target = std::fs::read_link(&path).unwrap_or_default();
            hash.feed(target.to_string_lossy().as_bytes());
        } else if kind.is_dir() {
            hash.feed(&nested_head(&path));
        } else if kind.is_file() {
            let within = files < caps.files && bytes.saturating_add(meta.len()) <= caps.bytes;
            match within.then(|| read_all(&path)).flatten() {
                Some(content) => {
                    files += 1;
                    bytes += content.len() as u64;
                    hash.feed(&content);
                }
                None => {
                    capped |= !within;
                    hash.feed(&meta.len().to_le_bytes());
                    let modified = meta
                        .modified()
                        .ok()
                        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                        .map_or(0, |since| since.as_nanos());
                    hash.feed(&modified.to_le_bytes());
                }
            }
        } else {
            tracing::debug!(path = %path.display(), "untracked entry is not a file, link or directory; skipped");
        }
    }
    if capped {
        tracing::info!(
            files = caps.files,
            bytes = caps.bytes,
            "untracked files past the content bound are fingerprinted by size and mtime"
        );
    }
    hash.0
}

fn read_all(path: &Path) -> Option<Vec<u8>> {
    let mut content = Vec::new();
    std::fs::File::open(path)
        .and_then(|mut file| file.read_to_end(&mut content))
        .ok()?;
    Some(content)
}

/// A nested repository's `HEAD`, and the commit a symbolic `HEAD` names when
/// its ref is a loose file: what moves when that repository moves.
fn nested_head(dir: &Path) -> Vec<u8> {
    let git = dir.join(".git");
    let head = std::fs::read(git.join("HEAD")).unwrap_or_default();
    let mut out = head.clone();
    if let Some(name) = String::from_utf8_lossy(&head).trim().strip_prefix("ref: ") {
        out.extend(std::fs::read(git.join(name.trim())).unwrap_or_default());
    }
    out
}
