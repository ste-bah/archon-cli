//! The paths a task's contract forbids it to change, as one matcher.
//!
//! # Why this is here (Issue-30)
//!
//! A task file's `## Files Forbidden to Change` section is parsed by the host
//! into a typed field, and until this module nothing read it. Live on
//! wf-719ff3b0 `agents-14-1`: the task named the crate's gate module,
//! `coverage.rs`, `data_lake.rs`, `data_store.rs`, `validation.rs` and the
//! provider adapters as forbidden; the coder edited the gate module, the
//! coverage module, a data-lake contract file and a data-store test, the
//! ownership grant admitted every one of them as an unclaimed in-scope change,
//! and all of it was declared and committed under the task.
//!
//! Two layers now read the list — the tool guard, which refuses a mutating
//! tool call before the file changes, and the capture-time grant, which
//! rejects a branch whose worktree changed a forbidden path anyway (a Bash
//! edit cannot be intercepted). Both must answer "is this path forbidden?"
//! identically, and they sit in crates with no edge between them
//! (`archon-tools` and `archon-workflow`), so the matcher lives below both,
//! here, for the same reason the overlap table does.
//!
//! # What an entry looks like, and what this makes of it
//!
//! The section is prose written for a coder, not a manifest. The live
//! entries were bullets like "`crates/x/src/gate.rs` and every other crate
//! module: `data_lake.rs`, `data_store.rs` (TASK-DL-002…009) — gate defects
//! are reported, never fixed", and "Frozen chain: `tasks/PRD-…/*` and
//! `prds/PRD-….md`", with paths wrapped across lines. So an entry is read as
//! follows, keyed on nothing but its own text:
//!
//! - Every backtick-quoted span is a candidate path; an entry with no
//!   backticks is one candidate as a whole.
//! - A candidate is trimmed, stripped of surrounding quotes, repaired where a
//!   line wrap split it after a `/`, cut at its first remaining whitespace
//!   (the `path — prose` and `path (…)` forms), stripped of trailing sentence
//!   punctuation and of a leading `./` or `/`.
//! - A trailing `/`, `/*` or `/**` makes a DIRECTORY prefix. `**/name` or a
//!   bare `name.ext` with no `/` makes a BASENAME rule, matching that file
//!   name anywhere: the live list named `coverage.rs` bare, and the file the
//!   coder changed was `crates/…/src/coverage.rs`. Any other `*` makes a
//!   GLOB where `*` spans any characters, `/` included. Anything else with a
//!   `/` is an exact FILE.
//! - A candidate with neither `/` nor `.` — "Frozen chain", "tests", "PRD
//!   files" — is prose and is ignored, as is one that is only wildcards.
//!
//! The rules err towards matching: a forbidden path that is missed lands in
//! the canonical tree under the task's name, which is the defect; a prose
//! fragment that is over-read as a path forbids a file nobody will touch.

use std::fmt::Write as _;

/// The most entries [`ForbiddenPaths::describe`] lists before eliding.
pub const DESCRIBED_ENTRIES: usize = 40;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Pattern {
    /// An exact repo-relative file.
    File(String),
    /// A repo-relative directory prefix, no trailing slash.
    Dir(String),
    /// A file name matched anywhere in the tree.
    Basename(String),
    /// The pattern split on `*`: a prefix, in-order middles, a suffix.
    Glob(Vec<String>),
}

impl Pattern {
    fn parse(candidate: &str) -> Option<Self> {
        let text = clean(candidate);
        if text.is_empty() || text.chars().all(|c| c == '*' || c == '/') {
            return None;
        }
        for suffix in ["/**", "/*", "/"] {
            if let Some(dir) = text.strip_suffix(suffix) {
                let dir = dir.trim_end_matches('/');
                return (!dir.is_empty() && !dir.contains('*')).then(|| Self::Dir(dir.to_string()));
            }
        }
        if let Some(name) = text.strip_prefix("**/")
            && !name.contains('/')
            && !name.contains('*')
        {
            return Some(Self::Basename(name.to_string()));
        }
        if text.contains('*') {
            let collapsed = text.replace("**", "*");
            return Some(Self::Glob(
                collapsed.split('*').map(str::to_string).collect(),
            ));
        }
        if text.contains('/') {
            return Some(Self::File(text));
        }
        text.contains('.').then_some(Self::Basename(text))
    }

    fn matches(&self, path: &str) -> bool {
        match self {
            Self::File(file) => path == file,
            Self::Dir(dir) => {
                path == dir
                    || path
                        .strip_prefix(dir.as_str())
                        .is_some_and(|r| r.starts_with('/'))
            }
            Self::Basename(name) => {
                path == name
                    || path
                        .strip_suffix(name.as_str())
                        .is_some_and(|r| r.ends_with('/'))
            }
            Self::Glob(segments) => glob_matches(segments, path),
        }
    }

    /// The wire form: an entry that [`Pattern::parse`] reads back as itself.
    fn wire(&self) -> String {
        match self {
            Self::File(file) => file.clone(),
            Self::Dir(dir) => format!("{dir}/"),
            Self::Basename(name) => format!("**/{name}"),
            Self::Glob(segments) => segments.join("*"),
        }
    }
}

/// A candidate as it is judged: quotes, wrap damage, trailing prose and
/// sentence punctuation removed; `./` and `/` prefixes dropped.
fn clean(candidate: &str) -> String {
    // A bullet wrapped after a `/` was joined with a space by the section
    // parser; the space is the wrap, not a separator.
    let mut repaired = String::with_capacity(candidate.len());
    let mut after_slash = false;
    for ch in candidate.trim().chars() {
        if after_slash && ch.is_whitespace() {
            continue;
        }
        after_slash = ch == '/';
        repaired.push(ch);
    }
    let head = repaired.split_whitespace().next().unwrap_or("");
    // Quotes and sentence punctuation nest either way round (`'x.rs',`), so
    // both are peeled until neither remains.
    let head = head
        .trim_end_matches(['`', '"', '\'', '.', ',', ';', ':', ')'])
        .trim_start_matches(['`', '"', '\'']);
    let head = head.strip_prefix("./").unwrap_or(head);
    head.trim_start_matches('/').to_string()
}

/// `*` spans any run of characters, `/` included: the first segment anchors
/// the start, the last anchors the end, the rest are found in order.
fn glob_matches(segments: &[String], path: &str) -> bool {
    let Some((first, rest)) = segments.split_first() else {
        return false;
    };
    let Some(mut remaining) = path.strip_prefix(first.as_str()) else {
        return false;
    };
    let Some((last, middles)) = rest.split_last() else {
        return remaining.is_empty();
    };
    for middle in middles {
        let Some(found) = remaining.find(middle.as_str()) else {
            return false;
        };
        remaining = &remaining[found + middle.len()..];
    }
    remaining.ends_with(last.as_str())
}

/// The union of a branch's forbidden entries, normalised once and matched by
/// either the tool guard or the capture-time grant.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ForbiddenPaths {
    patterns: Vec<Pattern>,
}

impl ForbiddenPaths {
    /// Read `entries` as the section's bullets, or as wire patterns: the wire
    /// form is an entry like any other, so a stamped list reads back to the
    /// same matcher.
    pub fn from_entries<I, S>(entries: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut patterns: Vec<Pattern> = Vec::new();
        for entry in entries {
            let entry = entry.as_ref();
            let spans: Vec<&str> = entry.split('`').skip(1).step_by(2).collect();
            let candidates: Vec<&str> = if spans.is_empty() { vec![entry] } else { spans };
            for candidate in candidates {
                if let Some(pattern) = Pattern::parse(candidate)
                    && !patterns.contains(&pattern)
                {
                    patterns.push(pattern);
                }
            }
        }
        Self { patterns }
    }

    pub fn is_empty(&self) -> bool {
        self.patterns.is_empty()
    }

    pub fn len(&self) -> usize {
        self.patterns.len()
    }

    /// The patterns in wire form, for stamping onto a request.
    pub fn patterns(&self) -> Vec<String> {
        self.patterns.iter().map(Pattern::wire).collect()
    }

    /// Whether `repo_relative` — as git or a tool names it, with or without a
    /// leading `./` — is a forbidden file, under a forbidden directory, a
    /// forbidden basename, or a glob match.
    pub fn matches(&self, repo_relative: &str) -> bool {
        let path = repo_relative.trim();
        let path = path
            .strip_prefix("./")
            .unwrap_or(path)
            .trim_start_matches('/');
        !path.is_empty() && self.patterns.iter().any(|pattern| pattern.matches(path))
    }

    /// The list as the preamble and a gap name it, bounded to
    /// [`DESCRIBED_ENTRIES`] entries plus a count of the rest.
    pub fn describe(&self) -> String {
        let mut out = self
            .patterns
            .iter()
            .take(DESCRIBED_ENTRIES)
            .map(Pattern::wire)
            .collect::<Vec<_>>()
            .join(", ");
        if self.patterns.len() > DESCRIBED_ENTRIES {
            let _ = write!(
                out,
                " …and {} more",
                self.patterns.len() - DESCRIBED_ENTRIES
            );
        }
        out
    }
}

#[cfg(test)]
#[path = "forbidden_paths_tests.rs"]
mod tests;
