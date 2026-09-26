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
//! - a glob, brace set or directory names every file it matches, up to
//!   [`PATTERN_CAP`]; one that matches more names nothing (it is too wide to
//!   be a finding about particular files).
//!
//! Only the listing decides; nothing of the text is trusted beyond being a
//! candidate.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::v2::verification::path_ownership::{DeclaredPathForm, declared_path_form};

/// Most files one pattern may name.
pub const PATTERN_CAP: usize = 10;
/// Most alternatives one brace set expands to.
const BRACE_CAP: usize = 32;

/// The regular files of the tree at `commit` (or of the working tree),
/// repository-relative.
pub fn tree_files(root: &Path, commit: Option<&str>) -> Arc<BTreeSet<String>> {
    static CACHE: Mutex<BTreeMap<(PathBuf, String), Arc<BTreeSet<String>>>> =
        Mutex::new(BTreeMap::new());
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
        for candidate in braces(&piece) {
            let Some(token) = super::residual_paths::strip_location(&candidate) else {
                continue;
            };
            let directory = token.ends_with('/');
            let relative = match declared_path_form(token, root) {
                DeclaredPathForm::Repo(path) => path,
                _ => continue,
            };
            if relative.starts_with('-')
                || relative.contains(['[', '\\'])
                || relative
                    .split('/')
                    .any(|segment| segment.is_empty() || segment == "." || segment == "..")
            {
                continue;
            }
            found.extend(matches(&relative, directory, files));
        }
    }
    found.into_iter().collect()
}

/// The files one clean candidate names.
fn matches(candidate: &str, directory: bool, files: &BTreeSet<String>) -> Vec<String> {
    let capped = |hits: Vec<String>| {
        if hits.len() <= PATTERN_CAP {
            hits
        } else {
            Vec::new()
        }
    };
    if candidate.contains(['*', '?']) {
        return capped(
            files
                .iter()
                .filter(|file| {
                    glob(candidate, file)
                        || file
                            .match_indices('/')
                            .any(|(at, _)| glob(candidate, &file[at + 1..]))
                })
                .cloned()
                .collect(),
        );
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
        return capped(inside);
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

/// Every expansion of the brace sets in `piece`, bounded.
fn braces(piece: &str) -> Vec<String> {
    let Some(open) = piece.find('{') else {
        return vec![piece.to_string()];
    };
    let Some(close) = piece[open..].find('}').map(|at| open + at) else {
        return vec![piece.to_string()];
    };
    let (head, body, tail) = (&piece[..open], &piece[open + 1..close], &piece[close + 1..]);
    let mut out = Vec::new();
    for alternative in body.split(',') {
        for rest in braces(tail) {
            out.push(format!("{head}{alternative}{rest}"));
            if out.len() >= BRACE_CAP {
                return out;
            }
        }
    }
    out
}

#[cfg(test)]
#[path = "residual_patterns_tests.rs"]
mod tests;
