//! Reading a test runner's output for the names of the tests that failed.
//!
//! Only a RUNNER's own lines are read — the `test <name> ... FAILED` result
//! lines and the `failures:` name block cargo's libtest harness prints, and
//! the `FAIL [ ... ] <binary> <name>` lines of nextest. Prose is never
//! scanned, for the same reason `context_output_test_counts` never scans it:
//! a sentence mentioning a test name is not a verdict on it.

/// How many trailing lines are kept for a command whose output names no
/// test — a non-cargo runner, or a build failure before any test ran.
pub(crate) const TAIL_LINES: usize = 40;

/// Whether `command` is one whose output [`failing_tests`] can read by name.
pub(crate) fn is_cargo_test_command(command: &str) -> bool {
    let lower = command.to_ascii_lowercase();
    lower.contains("cargo test") || lower.contains("cargo nextest")
}

/// The test ids `output` reports as failed, sorted and deduplicated.
///
/// Two sources are unioned. The per-test result line, `test a::b ... FAILED`
/// (a trailing timing annotation is tolerated), and the `failures:` block
/// libtest prints after the failure details: the line `failures:` followed by
/// four-space-indented names up to the first blank line. The block alone
/// would miss a harness killed mid-run; the result lines alone would miss
/// nothing, but the block is what a human reads and the two must agree.
pub(crate) fn failing_tests(output: &str) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    let mut in_block = false;
    for raw in output.lines() {
        let line = raw.trim_end_matches('\r');
        if let Some(id) = result_line_failure(line) {
            push_unique(&mut ids, id);
            in_block = false;
            continue;
        }
        if let Some(id) = nextest_failure(line) {
            push_unique(&mut ids, id);
            continue;
        }
        if line.trim() == "failures:" {
            in_block = true;
            continue;
        }
        if in_block {
            if line.trim().is_empty() {
                in_block = false;
                continue;
            }
            // Names in the block are indented; the detail section that
            // precedes it prints `---- name stdout ----` headers instead,
            // which this shape excludes.
            if let Some(name) = line.strip_prefix("    ")
                && is_test_id(name.trim())
            {
                push_unique(&mut ids, name.trim().to_string());
            } else {
                in_block = false;
            }
        }
    }
    ids.sort();
    ids
}

/// `test <id> ... FAILED` — libtest's per-test verdict line.
fn result_line_failure(line: &str) -> Option<String> {
    let rest = line.trim_start().strip_prefix("test ")?;
    let (id, verdict) = rest.split_once(" ... ")?;
    let verdict = verdict.trim();
    if !(verdict == "FAILED" || verdict.starts_with("FAILED ")) {
        return None;
    }
    let id = id.trim();
    is_test_id(id).then(|| id.to_string())
}

/// `FAIL [   0.012s] crate-name path::to::test` — nextest's verdict line.
fn nextest_failure(line: &str) -> Option<String> {
    let rest = line.trim_start().strip_prefix("FAIL [")?;
    let (_, after) = rest.split_once(']')?;
    let mut parts = after.split_whitespace();
    let _binary = parts.next()?;
    let id = parts.next()?;
    (parts.next().is_none() && is_test_id(id)).then(|| id.to_string())
}

/// A libtest id: path segments joined by `::`, no whitespace. A bare word
/// (`should_fail`) is a valid id for a test at a crate root.
fn is_test_id(candidate: &str) -> bool {
    !candidate.is_empty()
        && !candidate.contains(char::is_whitespace)
        && candidate
            .split("::")
            .all(|segment| !segment.is_empty() && segment.chars().all(|ch| ch.is_alphanumeric() || ch == '_'))
}

fn push_unique(ids: &mut Vec<String>, id: String) {
    if !ids.contains(&id) {
        ids.push(id);
    }
}

/// The last [`TAIL_LINES`] lines of `output`, trimmed of trailing whitespace.
pub(crate) fn tail(output: &str) -> Vec<String> {
    let lines: Vec<&str> = output.lines().collect();
    let start = lines.len().saturating_sub(TAIL_LINES);
    lines[start..]
        .iter()
        .map(|line| line.trim_end().to_string())
        .collect()
}

/// The package a cargo test command names, if it names one: `-p x`,
/// `--package x`, `-p=x`, `--package=x`. `None` when the command runs the
/// current package (the workspace root's) or the whole workspace.
pub(crate) fn cargo_package(command: &str) -> Option<String> {
    let tokens: Vec<&str> = command.split_whitespace().collect();
    for (index, token) in tokens.iter().enumerate() {
        for flag in ["-p", "--package"] {
            if *token == flag {
                return tokens.get(index + 1).map(|name| name.trim_matches('"').to_string());
            }
            if let Some(value) = token.strip_prefix(&format!("{flag}=")) {
                return Some(value.trim_matches('"').to_string());
            }
        }
    }
    None
}

#[cfg(test)]
#[path = "test_baseline_parse_tests.rs"]
mod tests;
