//! Batch E2: which `path:line` references in a failing check's output are
//! the failure itself.
//!
//! Batch E read every `path:line` token in a check's output. Live on
//! wf-0ddadd81 a check that built the binary before failing with a
//! location-less `Error: ...` carried a page of rustc `warning: ... is never
//! used --> src/...:5` blocks, and every one of those harness files was
//! implicated. A warning compiled; it is not why the check failed.
//!
//! So locations are taken from a positive allowlist of failure signals only:
//!
//! - a panic's location (`thread '...' panicked at path:12:5`);
//! - the primary span (`--> path:12:5`) of an error-level diagnostic block
//!   (`error:` / `error[E0425]:`), never of its `note:`/`help:` children;
//! - a stack frame: `at path:12:5` / `at f (path:12:5)` (Rust backtraces,
//!   JavaScript), `File "path", line 12` (Python), `path:12: in f` (pytest);
//! - a line that opens with a location whose message is error-level
//!   (`path:12:5: error: ...`, `path:12: AssertionError`, `... FAILED`).
//!
//! Everything else is not read: `warning`, `note`, `help` and `info` blocks
//! (rustc/cargo, clippy, linker messages) up to the blank line or unindented
//! text that ends them; lint and grep-style listings; a span whose block
//! header was truncated away. Output that names no failure location
//! implicates nothing, and routing falls back to the check's owners.

use std::path::Path;

use super::script::residual_paths::is_repo_file;

/// What diagnostic block a line belongs to.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Block {
    /// Not inside a diagnostic block.
    Plain,
    /// An error-level diagnostic: its primary span is a failure location.
    Error,
    /// A warning, note, help or info diagnostic: nothing in it is read.
    Noise,
}

/// Words a location-led line's message opens with that make it error-level.
const ERROR_WORDS: [&str; 9] = [
    "error",
    "fatal",
    "panic",
    "panicked",
    "fail",
    "failed",
    "failure",
    "assert",
    "assertion",
];

/// The block an unindented line opens, when it is a diagnostic header
/// (`warning: ...`, `error[E0425]: ...`, `note: ...`).
fn header(line: &str) -> Option<Block> {
    let word: String = line
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect();
    let next = line[word.len()..].chars().next();
    if !matches!(next, Some(':' | '[' | '(')) {
        return None;
    }
    match word.to_ascii_lowercase().as_str() {
        "error" => Some(Block::Error),
        "warning" | "warn" | "note" | "help" | "info" | "hint" => Some(Block::Noise),
        _ => None,
    }
}

/// Whether an unindented line continues the diagnostic block above it: a
/// rustc gutter (`12 | ...`, `|`, `= note`), a span (`-->`, `:::`) or an
/// elision (`...`).
fn continues(line: &str) -> bool {
    let digits = line.bytes().take_while(u8::is_ascii_digit).count();
    let after_digits = line[digits..].trim_start();
    ["|", "=", "-->", ":::", "..."]
        .iter()
        .any(|prefix| line.starts_with(prefix))
        || (digits > 0 && after_digits.starts_with('|'))
}

/// Whether `word` (a message's first word) is error-level.
fn error_level(word: &str) -> bool {
    let word = word.trim_matches(|c: char| !c.is_ascii_alphanumeric());
    let lower = word.to_ascii_lowercase();
    ERROR_WORDS.contains(&lower.as_str())
        || word.ends_with("Error")
        || word.ends_with("Exception")
        || word.starts_with("Assert")
}

/// The failure locations one line outside a noise block names.
fn signal_locations(line: &str, root: &Path) -> Vec<String> {
    let trimmed = line.trim_start();
    let located = |text: &str| -> Vec<String> {
        text.split_whitespace()
            .filter_map(|token| located_file(token, root))
            .collect()
    };
    if let Some(at) = line.find("panicked at") {
        return located(&line[at..]);
    }
    let mut tokens = trimmed.split_whitespace();
    let Some(first) = tokens.next() else {
        return Vec::new();
    };
    if first == "at" {
        return located(&trimmed[2..]);
    }
    if first == "File" {
        return python_frame(trimmed, root).into_iter().collect();
    }
    let Some(file) = located_file(first, root) else {
        return Vec::new();
    };
    match tokens.next() {
        Some("in") => vec![file],
        Some(word) if error_level(word) => vec![file],
        _ => Vec::new(),
    }
}

/// The file a Python frame names: `File "path", line 12, in f`.
fn python_frame(line: &str, root: &Path) -> Option<String> {
    let rest = line.strip_prefix("File \"")?;
    let (path, rest) = rest.split_once('"')?;
    let number: String = rest
        .trim_start_matches([',', ' '])
        .strip_prefix("line ")?
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    located_file(&format!("{path}:{number}"), root)
}

/// The distinct repository files `text` names as failure locations (see the
/// module docs), last first, at most `limit`.
pub fn failure_locations(text: &str, root: &Path, limit: usize) -> Vec<String> {
    let mut lines: Vec<Vec<String>> = Vec::new();
    let mut block = Block::Plain;
    for line in text.lines() {
        if line.trim().is_empty() {
            block = Block::Plain;
            continue;
        }
        let indented = line.starts_with(char::is_whitespace);
        if !indented {
            if let Some(opened) = header(line) {
                block = opened;
                continue;
            }
            if !continues(line) {
                block = Block::Plain;
            }
        }
        match block {
            Block::Noise => {}
            Block::Error if line.trim_start().starts_with("-->") => {
                let span = line.trim_start().trim_start_matches("-->");
                lines.push(
                    span.split_whitespace()
                        .filter_map(|t| located_file(t, root))
                        .collect(),
                );
            }
            Block::Error | Block::Plain => lines.push(signal_locations(line, root)),
        }
    }
    let mut found: Vec<String> = Vec::new();
    for file in lines
        .into_iter()
        .rev()
        .flat_map(|files| files.into_iter().rev())
    {
        if found.len() >= limit {
            break;
        }
        if !found.contains(&file) {
            found.push(file);
        }
    }
    found
}

/// The repository-relative file a token names with a line location
/// (`path:12`, `path:12:4`, `(path:12:4)`), when it exists under `root`.
pub(crate) fn located_file(token: &str, root: &Path) -> Option<String> {
    let token = token.trim_matches(|c: char| {
        matches!(
            c,
            '(' | ')' | '[' | ']' | '<' | '>' | '"' | '\'' | '`' | ',' | ';'
        )
    });
    let token = archon_write_plan::lexical_path::portable(token);
    // The drive colon is part of the path, not its line location.
    let drive = token.as_bytes().get(1) == Some(&b':') && token.as_bytes()[0].is_ascii_alphabetic();
    let offset = if drive { 2 } else { 0 };
    let colon = token[offset..].find(':')? + offset;
    let (head, tail) = (&token[..colon], &token[colon + 1..]);
    let line = tail.split(':').next().unwrap_or_default();
    if line.is_empty() || !line.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let rooted = archon_write_plan::lexical_path::under_root(head, &root.to_string_lossy());
    let relative = rooted.as_deref().unwrap_or(head);
    let relative = relative.strip_prefix("./").unwrap_or(relative);
    let parts: Vec<&str> = relative.split('/').collect();
    let clean = |parts: &[&str]| {
        !parts.is_empty()
            && parts
                .iter()
                .all(|p| !p.is_empty() && *p != "." && *p != "..")
    };
    if !archon_write_plan::lexical_path::rooted(relative) {
        return (clean(&parts) && is_repo_file(root, relative)).then(|| relative.to_string());
    }
    // An absolute path under some other copy of the repository (a scratch
    // checkout the check ran in): its longest tail of two or more
    // components that is a repository file.
    (1..parts.len().saturating_sub(1)).find_map(|at| {
        let tail = &parts[at..];
        let candidate = tail.join("/");
        (clean(tail) && is_repo_file(root, &candidate)).then_some(candidate)
    })
}

#[cfg(test)]
#[path = "acceptance_signals_tests.rs"]
mod tests;
