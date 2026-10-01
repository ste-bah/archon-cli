//! Issue-117: the repository files a finding's text names, patterns
//! included, resolved against the host's own listing of the tree.
//!
//! A verifier names files the way a reader would: an exact path, a short
//! form (`providers/x_store.rs`), a glob (`providers/*_store.rs`), a brace
//! set (`providers/{a,b}_store.rs`) or a directory. Each candidate is
//! resolved to the exact regular files the tree holds -- at the commit the
//! recording verifier judged when git can read it, else the working tree:
//!
//! - an exact repository path names itself;
//! - a short form names the one file whose path ends with it (none when it
//!   is ambiguous);
//! - a glob, brace set or directory names EVERY file it matches (Batch O:
//!   a pattern wider than a bound used to name nothing, so its gap lost its
//!   routing and was reported instead of planned).
//!
//! Only the listing decides; nothing of the text is trusted beyond being a
//! candidate.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::v2::verification::path_ownership::{DeclaredPathForm, declared_path_form};

/// HARNESS BOUND (Batch O2, CUT-12) on work only, never on what is named:
/// the most alternatives one piece's brace sets are expanded to. Past it the
/// piece is not expanded at all; each tree file is matched against it
/// directly (`residual_patterns_unexpanded`), which names exactly the files
/// the full expansion would.
pub(super) const BRACE_EXPANSION_BOUND: usize = 1_024;

/// Each tree's regular files, by (repository root, commit).
type TreeCache = BTreeMap<(PathBuf, String), Arc<BTreeSet<String>>>;

/// The regular files of the tree at `commit` (or of the working tree),
/// repository-relative.
pub fn tree_files(root: &Path, commit: Option<&str>) -> Arc<BTreeSet<String>> {
    static CACHE: Mutex<TreeCache> = Mutex::new(BTreeMap::new());
    if let Some(commit) = commit.filter(|commit| commit_exists(root, commit)) {
        let key = (root.to_path_buf(), commit.to_string());
        if let Some(hit) = CACHE.lock().ok().and_then(|cache| cache.get(&key).cloned()) {
            return hit;
        }
        let files = Arc::new(committed_files(root, commit));
        if let Ok(mut cache) = CACHE.lock() {
            cache.insert(key, files.clone());
        }
        return files;
    }
    Arc::new(working_files(root))
}

fn git_out(root: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .ok()?;
    out.status.success().then_some(out.stdout)
}

fn commit_exists(root: &Path, commit: &str) -> bool {
    !commit.starts_with('-')
        && git_out(root, &["cat-file", "-e", &format!("{commit}^{{commit}}")]).is_some()
}

/// Blobs of the tree at `commit`: never a link or a submodule.
fn committed_files(root: &Path, commit: &str) -> BTreeSet<String> {
    let Some(out) = git_out(root, &["ls-tree", "-r", "-z", commit]) else {
        return BTreeSet::new();
    };
    out.split(|b| *b == 0)
        .filter_map(|entry| {
            let entry = String::from_utf8_lossy(entry);
            let (meta, name) = entry.split_once('\t')?;
            (meta.starts_with("100644 blob") || meta.starts_with("100755 blob"))
                .then(|| name.to_string())
        })
        .collect()
}

/// Regular files of the working tree: git's tracked and untracked
/// (not ignored) files when it is a checkout, else a walk; links and paths
/// outside the root never count.
fn working_files(root: &Path) -> BTreeSet<String> {
    if let Some(out) = git_out(
        root,
        &[
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ],
    ) {
        return out
            .split(|b| *b == 0)
            .map(|name| String::from_utf8_lossy(name).to_string())
            .filter(|name| !name.is_empty() && super::residual_paths::is_repo_file(root, name))
            .collect();
    }
    let mut found = BTreeSet::new();
    let mut stack = vec![PathBuf::new()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(root.join(&dir)) else {
            continue;
        };
        for entry in entries.flatten() {
            let relative = dir.join(entry.file_name());
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() && entry.file_name() != ".git" {
                stack.push(relative);
            } else if kind.is_file() {
                found.insert(relative.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    found
}

/// The files `text` names in `files` (the tree of `root`), sorted.
pub fn resolve_named(text: &str, root: &Path, files: &BTreeSet<String>) -> Vec<String> {
    let mut found = BTreeSet::new();
    for piece in tokens(text) {
        let Some(candidates) = braces(&piece) else {
            // Past the bound: every file the expansion would name, found
            // without expanding it.
            found.extend(unexpanded::named_by(&piece, root, files));
            continue;
        };
        for candidate in candidates {
            if let Some((relative, directory)) = clean_candidate(&candidate, root) {
                found.extend(matches(&relative, directory, files));
            }
        }
    }
    found.into_iter().collect()
}

/// A candidate as the repository-relative path it names, and whether it
/// names a directory; `None` for anything that is no clean repository path.
fn clean_candidate(candidate: &str, root: &Path) -> Option<(String, bool)> {
    let token = super::residual_paths::strip_location(candidate)?;
    let directory = token.ends_with('/');
    let relative = match declared_path_form(token, root) {
        DeclaredPathForm::Repo(path) => path,
        _ => return None,
    };
    let clean = !relative.starts_with('-')
        && !relative.contains(['[', '\\'])
        && !relative
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..");
    clean.then_some((relative, directory))
}

/// Whether `text` holds no glob character (`matches` reads it literally).
fn glob_free(text: &str) -> bool {
    !text.contains(['*', '?'])
}

/// The files one clean candidate names.
fn matches(candidate: &str, directory: bool, files: &BTreeSet<String>) -> Vec<String> {
    if !glob_free(candidate) {
        return files
            .iter()
            .filter(|file| {
                glob(candidate, file)
                    || file
                        .match_indices('/')
                        .any(|(at, _)| glob(candidate, &file[at + 1..]))
            })
            .cloned()
            .collect();
    }
    if !directory && files.contains(candidate) {
        return vec![candidate.to_string()];
    }
    let under = format!("{candidate}/");
    let inside: Vec<String> = files
        .iter()
        .filter(|file| file.starts_with(&under))
        .cloned()
        .collect();
    if !inside.is_empty() {
        return inside;
    }
    if directory {
        return Vec::new();
    }
    // A short form: exactly one file ends with it.
    let suffix = format!("/{candidate}");
    let mut short = files.iter().filter(|file| file.ends_with(&suffix));
    match (short.next(), short.next()) {
        (Some(only), None) => vec![only.clone()],
        _ => Vec::new(),
    }
}

/// `*` within a segment, `**` across segments, `?` one character.
fn glob(pattern: &str, path: &str) -> bool {
    fn go(p: &[u8], s: &[u8]) -> bool {
        match p.first() {
            None => s.is_empty(),
            Some(b'*') if p.get(1) == Some(&b'*') => {
                let rest = p[2..].strip_prefix(b"/").unwrap_or(&p[2..]);
                (0..=s.len()).any(|at| (at == 0 || s[at - 1] == b'/') && go(rest, &s[at..]))
                    || (0..=s.len()).any(|at| go(&p[2..], &s[at..]))
            }
            Some(b'*') => (0..=s.len())
                .take_while(|at| *at == 0 || s[at - 1] != b'/')
                .any(|at| go(&p[1..], &s[at..])),
            Some(b'?') => s.first().is_some_and(|c| *c != b'/') && go(&p[1..], &s[1..]),
            Some(c) => s.first() == Some(c) && go(&p[1..], &s[1..]),
        }
    }
    go(pattern.as_bytes(), path.as_bytes())
}

/// Path-shaped pieces of `text`: split on whitespace, quotes and brackets,
/// and on commas outside a brace set.
fn tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;
    for c in text.chars() {
        let split = c.is_whitespace()
            || "()[]'\"`<>=|;".contains(c)
            || (c == ',' && depth == 0)
            || (c == '}' && depth == 0);
        match c {
            '{' => depth += 1,
            '}' if depth > 0 => depth -= 1,
            _ => {}
        }
        if split {
            if !current.is_empty() {
                out.push(std::mem::take(&mut current));
            }
            depth = 0;
        } else {
            current.push(c);
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// Every expansion of the brace sets in `piece`; `None` when there are more
/// than [`BRACE_EXPANSION_BOUND`] (the caller then matches without them).
fn braces(piece: &str) -> Option<Vec<String>> {
    let Some(open) = piece.find('{') else {
        return Some(vec![piece.to_string()]);
    };
    let Some(close) = piece[open..].find('}').map(|at| open + at) else {
        return Some(vec![piece.to_string()]);
    };
    let (head, body, tail) = (&piece[..open], &piece[open + 1..close], &piece[close + 1..]);
    let rests = braces(tail)?;
    let mut out = Vec::new();
    for alternative in body.split(',') {
        for rest in &rests {
            if out.len() >= BRACE_EXPANSION_BOUND {
                return None;
            }
            out.push(format!("{head}{alternative}{rest}"));
        }
    }
    Some(out)
}

#[path = "residual_patterns_unexpanded.rs"]
mod unexpanded;

#[cfg(test)]
#[path = "residual_patterns_tests.rs"]
mod tests;
