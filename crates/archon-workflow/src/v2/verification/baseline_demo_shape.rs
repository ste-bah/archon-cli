//! The one command shape the host reads as a fail-on-old demonstration
//! (Batch K, I3; see `baseline_demo`).
//!
//! Deliberately narrow. Everything the host exempts must provably have run
//! against a tree of its own, so a shape it cannot read end to end is never
//! a demonstration and the failure counts against the change:
//!
//! ```text
//! [prep && ...] git [-C <repo>] archive [opts] <rev> [paths] | tar <opts> -C <dir>
//!     [&& prep ...] && cd <dir> && <check>
//! ```
//!
//! - `prep` is only `mkdir`, `rm`, `cp`, `mv`, `touch`, `chmod` or `true`:
//!   nothing that could run the check against the checkout;
//! - `tar` is the immediate and only consumer of the archive;
//! - `<dir>` is absolute, has no `.`/`..` component, is not `/` and lies
//!   outside the repository;
//! - `<check>` never leaves `<dir>` or reaches into the repository: no
//!   `cd`/`pushd`/`popd`, no shell, `eval`, `exec`, `env` or `source`, no
//!   `-C`/`--manifest-path`/`--directory`/`--cwd`/`--git-dir`/`--work-tree`
//!   argument, and no absolute path under the repository;
//! - the whole command has exactly one `git` word, no `$` (variables,
//!   command substitution), no backticks, no newline, and no `||`, `&`,
//!   `(`, `)`, `<` or `>`.

use std::path::Path;

/// A parsed demonstration: what it materialized, where, and the check it
/// then ran there (its words, re-joined).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Shape {
    pub(crate) rev: String,
    pub(crate) dir: String,
    pub(crate) check: String,
}

const PREP: &[&str] = &["mkdir", "rm", "cp", "mv", "touch", "chmod", "true"];
const LEAVES: &[&str] = &[
    "cd", "pushd", "popd", "eval", "exec", "source", ".", "sh", "bash", "zsh", "dash", "ksh",
    "env", "sudo", "xargs",
];
const REDIRECTING_FLAGS: &[&str] = &[
    "-C",
    "--manifest-path",
    "--directory",
    "--cwd",
    "--git-dir",
    "--work-tree",
];

/// Shell words and operators, quotes removed.
pub(crate) fn words(command: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut word = String::new();
    let mut quote: Option<char> = None;
    let mut chars = command.chars().peekable();
    let flush = |word: &mut String, out: &mut Vec<String>| {
        if !word.is_empty() {
            out.push(std::mem::take(word));
        }
    };
    while let Some(c) = chars.next() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => word.push(c),
            None if c == '\'' || c == '"' => quote = Some(c),
            None if c.is_whitespace() => flush(&mut word, &mut out),
            None if matches!(c, '|' | '&' | ';' | '(' | ')' | '>' | '<') => {
                flush(&mut word, &mut out);
                let mut op = c.to_string();
                if let Some(&next) = chars.peek()
                    && (c == '|' || c == '&' || c == '>')
                    && next == c
                {
                    op.push(next);
                    chars.next();
                }
                out.push(op);
            }
            None => word.push(c),
        }
    }
    flush(&mut word, &mut out);
    out
}

fn base(word: &str) -> &str {
    Path::new(word)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(word)
}

fn under(path: &str, root: &Path) -> bool {
    let path = Path::new(path);
    path.starts_with(root)
        || root
            .canonicalize()
            .map(archon_shell::paths::plain)
            .is_ok_and(|real| path.starts_with(real))
}

fn clean_dir(dir: &str, repo: &Path) -> bool {
    // The demonstration is POSIX shell, so `dir` is judged as a POSIX path on
    // every host: `Path::is_absolute` is false for `/tmp/x` on Windows, and
    // `std::path::Component` parsing is platform-dependent (Issue-234).
    let segments: Vec<&str> = dir.split('/').filter(|s| !s.is_empty()).collect();
    dir.starts_with('/')
        && !segments.is_empty()
        && segments.iter().all(|s| *s != "." && *s != "..")
        && !under(dir, repo)
        && !repo.starts_with(Path::new(dir))
}

/// `git [-C repo] archive [opts] <rev> [paths] | tar ... -C <dir>`.
fn materialization(segment: &[String], repo: &Path) -> Option<(String, String)> {
    let pipe = segment.iter().position(|w| w == "|")?;
    let (git, tar) = (&segment[..pipe], &segment[pipe + 1..]);
    if tar.iter().any(|w| w == "|") || base(tar.first()?) != "tar" || base(git.first()?) != "git" {
        return None;
    }
    let mut k = 1;
    while k < git.len() && git[k] != "archive" {
        match git[k].as_str() {
            "-C" => {
                let named = Path::new(git.get(k + 1)?);
                let same = named.canonicalize().map(archon_shell::paths::plain).ok()?
                    == repo.canonicalize().map(archon_shell::paths::plain).ok()?;
                if !same {
                    return None;
                }
                k += 2;
            }
            "-c" => k += 2,
            _ => return None,
        }
    }
    let mut k = k + 1;
    let rev = loop {
        match git.get(k)?.as_str() {
            "--format" | "--prefix" => k += 2,
            w if w.starts_with("--format=") || w.starts_with("--prefix=") => k += 1,
            w if w.starts_with('-') => return None,
            w => break w.to_string(),
        }
    };
    let mut dir = None;
    let mut m = 1;
    while m < tar.len() {
        let w = tar[m].as_str();
        if let Some(d) = w.strip_prefix("--directory=") {
            dir = Some(d.to_string());
        } else if w == "--directory"
            || (w.starts_with('-') && !w.starts_with("--") && w.ends_with('C'))
        {
            dir = Some(tar.get(m + 1)?.clone());
            m += 1;
        }
        m += 1;
    }
    Some((rev, dir?))
}

/// A preparation step: an allowed command whose every operand is an
/// absolute path outside the repository. It runs in the checkout, so a
/// relative operand (or `~`, or `..`) would carry the change under review
/// into the old tree.
fn is_prep(segment: &[String], repo: &Path) -> bool {
    let Some((first, operands)) = segment.split_first() else {
        return false;
    };
    PREP.contains(&base(first))
        && operands.iter().all(|w| {
            w.starts_with('-') || (w.starts_with('/') && !w.contains("..") && !under(w, repo))
        })
}

/// The demonstration `command` is, if it has exactly the shape above.
pub(crate) fn shape(command: &str, repo: &Path) -> Option<Shape> {
    if command.contains(['$', '`', '\n', '\r']) {
        return None;
    }
    let words = words(command);
    let forbidden = ["||", "&", "(", ")", "<", ">", ">>"];
    if words.iter().any(|w| forbidden.contains(&w.as_str()))
        || words.iter().filter(|w| base(w) == "git").count() != 1
    {
        return None;
    }
    let segments: Vec<&[String]> = words
        .split(|w| w == "&&" || w == ";")
        .filter(|segment| !segment.is_empty())
        .collect();
    let at = segments
        .iter()
        .position(|segment| segment.iter().any(|w| base(w) == "git"))?;
    let (rev, dir) = materialization(segments[at], repo)?;
    if !clean_dir(&dir, repo) || !segments[..at].iter().all(|s| is_prep(s, repo)) {
        return None;
    }
    let cd = at
        + 1
        + segments[at + 1..]
            .iter()
            .position(|segment| segment.first().is_some_and(|w| w == "cd"))?;
    let enters = segments[cd].len() == 2
        && segments[cd][1].trim_end_matches('/') == dir.trim_end_matches('/');
    if !enters || !segments[at + 1..cd].iter().all(|s| is_prep(s, repo)) {
        return None;
    }
    let check: Vec<&String> = segments[cd + 1..].iter().flat_map(|s| s.iter()).collect();
    let escapes = check.iter().any(|w| {
        LEAVES.contains(&base(w))
            || REDIRECTING_FLAGS
                .iter()
                .any(|flag| w.as_str() == *flag || w.starts_with(&format!("{flag}=")))
            || w.contains("..")
            || w.starts_with('~')
            || under(w, repo)
    });
    if check.is_empty() || escapes {
        return None;
    }
    let check = segments[cd + 1..]
        .iter()
        .map(|s| s.join(" "))
        .collect::<Vec<_>>()
        .join(" && ");
    Some(Shape { rev, dir, check })
}

/// `command`'s words re-joined, for matching a check across records.
pub(crate) fn normalized(command: &str) -> String {
    words(command).join(" ")
}
