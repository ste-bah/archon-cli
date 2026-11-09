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
#[cfg(test)]
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

/// The test ids `output` reports as PASSED, sorted and deduplicated: the
/// libtest verdict line `test <id> ... ok` and nextest's `PASS [ .. ]
/// <binary> <id>`. An ignored test (`... ignored`, nextest `SKIP`) is not
/// passed; a quiet run that prints no per-test line names none.
pub(crate) fn passed_tests(output: &str) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    for raw in output.lines() {
        let line = raw.trim_end_matches('\r');
        if let Some(id) = result_line_verdict(line, "ok").or_else(|| nextest_pass(line)) {
            push_unique(&mut ids, id);
        }
    }
    ids.sort();
    ids
}

/// The test ids `output` reports as IGNORED (`test <id> ... ignored`, a
/// reason after it tolerated; nextest `SKIP`), sorted and deduplicated.
pub(crate) fn ignored_tests(output: &str) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    for raw in output.lines() {
        let line = raw.trim_end_matches('\r');
        let skipped = || {
            let rest = line.trim_start().strip_prefix("SKIP [")?;
            let (_, after) = rest.split_once(']')?;
            let parts: Vec<&str> = after
                .split_whitespace()
                .filter(|part| !(part.starts_with('(') || part.ends_with(')')))
                .collect();
            let [_binary, id] = parts.as_slice() else {
                return None;
            };
            is_test_id(id).then(|| (*id).to_string())
        };
        if let Some(id) = result_line_verdict(line, "ignored").or_else(skipped) {
            push_unique(&mut ids, id);
        }
    }
    ids.sort();
    ids
}

/// `test <id> ... <verdict>` for exactly `verdict` (a trailing timing
/// annotation tolerated).
fn result_line_verdict(line: &str, verdict: &str) -> Option<String> {
    let rest = line.trim_start().strip_prefix("test ")?;
    let (id, said) = rest.split_once(" ... ")?;
    let said = said.trim();
    if !(said == verdict
        || said.starts_with(&format!("{verdict} "))
        || said.starts_with(&format!("{verdict},")))
    {
        return None;
    }
    let id = id.trim();
    is_test_id(id).then(|| id.to_string())
}

/// `PASS [   0.012s] crate-name path::to::test` -- nextest's pass line, a
/// progress counter such as `(1/10)` tolerated before the binary.
fn nextest_pass(line: &str) -> Option<String> {
    let rest = line.trim_start().strip_prefix("PASS [")?;
    let (_, after) = rest.split_once(']')?;
    let parts: Vec<&str> = after
        .split_whitespace()
        .filter(|part| !(part.starts_with('(') || part.ends_with(')')))
        .collect();
    let [_binary, id] = parts.as_slice() else {
        return None;
    };
    is_test_id(id).then(|| (*id).to_string())
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
        && candidate.split("::").all(|segment| {
            !segment.is_empty() && segment.chars().all(|ch| ch.is_alphanumeric() || ch == '_')
        })
}

fn push_unique(ids: &mut Vec<String>, id: String) {
    if !ids.contains(&id) {
        ids.push(id);
    }
}

/// The repo-relative files a failed cargo-style command's ERROR diagnostics
/// point at, sorted and deduplicated: the `--> path:line:col` location under
/// an `error` header (rustc, clippy; a warning promoted by `-D warnings` is
/// printed as an error), and rustfmt's `Diff in <path>:<line>:` line. A
/// location under a `warning`/`note`/`help` header is not a failure and is
/// not read. Paths are relative to `worktree` when absolute (the working
/// directory the command ran in), and kept as printed when relative; an
/// absolute path under another root (a registry crate) is dropped.
pub(crate) fn diagnostic_files(output: &str, worktree: &std::path::Path) -> Vec<String> {
    let roots: Vec<std::path::PathBuf> = {
        let given = worktree.to_path_buf();
        let canonical = std::fs::canonicalize(&given)
            .map(archon_shell::paths::plain)
            .unwrap_or_else(|_| given.clone());
        let mut roots = vec![given];
        if !roots.contains(&canonical) {
            roots.push(canonical);
        }
        roots
    };
    let mut files: Vec<String> = Vec::new();
    let located = crate::located_diagnostics::located_lines(output);
    for diagnostic in &located {
        if diagnostic.kind == crate::located_diagnostics::Kind::Error {
            if let Some(relative) = relative_to(&diagnostic.path, &roots) {
                files.push(relative);
            }
        }
    }
    for line in output.lines() {
        if let Some(path) = line
            .strip_prefix("Diff in ")
            .and_then(|rest| strip_line_col(rest.trim().trim_end_matches(':')))
        {
            if let Some(relative) = relative_to(&path, &roots) {
                files.push(relative);
            }
        }
    }
    files.sort();
    files.dedup();
    files
}

/// `path:line:col` or `path:line` → `path`; a bare path is returned as is.
/// Only trailing numeric segments are stripped, so a Windows drive letter
/// survives.
fn strip_line_col(location: &str) -> Option<String> {
    let mut path = location;
    for _ in 0..2 {
        if let Some((head, tail)) = path.rsplit_once(':')
            && !tail.is_empty()
            && tail.chars().all(|c| c.is_ascii_digit())
        {
            path = head;
        }
    }
    (!path.is_empty()).then(|| path.to_string())
}

fn relative_to(path: &str, roots: &[std::path::PathBuf]) -> Option<String> {
    let unified = archon_write_plan::lexical_path::portable(path);
    if archon_write_plan::lexical_path::rooted(&unified) {
        return roots.iter().find_map(|root| {
            archon_write_plan::lexical_path::under_root(&unified, &root.to_string_lossy())
        });
    }
    (!unified.split('/').any(|part| part == ".."))
        .then(|| unified.trim_start_matches("./").to_string())
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
                return tokens
                    .get(index + 1)
                    .map(|name| name.trim_matches('"').to_string());
            }
            if let Some(value) = token.strip_prefix(&format!("{flag}=")) {
                return Some(value.trim_matches('"').to_string());
            }
        }
    }
    None
}

/// Whether every started test binary printed a readable summary. This records
/// only observed test ids: a fail-fast stop can leave later binaries unstarted,
/// so matching summaries do not establish a complete test inventory.
pub(crate) fn harness_reported(output: &str) -> bool {
    let lines: Vec<&str> = output.lines().map(str::trim_start).collect();
    if lines.iter().any(|line| line.starts_with("Summary [")) {
        return true;
    }
    let headers = lines
        .iter()
        .filter(|line| {
            line.strip_prefix("running ")
                .and_then(|rest| {
                    rest.strip_suffix(" tests")
                        .or_else(|| rest.strip_suffix(" test"))
                })
                .is_some_and(|n| n.parse::<usize>().is_ok())
        })
        .count();
    let results = lines
        .iter()
        .filter(|line| line.starts_with("test result: "))
        .count();
    results > 0 && results == headers
}

/// The failures the harness counted: the `N failed` of every libtest
/// `test result:` line summed, or nextest's `Summary` count. `None` when a
/// summary line carries no count the host can read.
pub(crate) fn failed_count(output: &str) -> Option<usize> {
    let summaries: Vec<&str> = output
        .lines()
        .map(str::trim_start)
        .filter(|line| line.starts_with("test result: ") || line.starts_with("Summary ["))
        .collect();
    if summaries.is_empty() {
        return None;
    }
    let mut total = 0usize;
    for line in summaries {
        let words: Vec<&str> = line
            .split(|c: char| c.is_whitespace() || c == ';' || c == ',')
            .filter(|w| !w.is_empty())
            .collect();
        let count = words
            .windows(2)
            .find(|pair| pair[1] == "failed")
            .and_then(|pair| pair[0].parse::<usize>().ok());
        match count {
            Some(count) => total += count,
            // A summary with no failure count (all passed on nextest).
            None if !line.contains("failed") => {}
            None => return None,
        }
    }
    Some(total)
}

#[cfg(test)]
#[path = "test_baseline_parse_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "test_baseline_parse_rule_tests.rs"]
mod rule_tests;

/// Failure markers with unstable run details masked; absent markers prove nothing.
pub(crate) fn failure_identity(output: &str, root: &std::path::Path) -> Option<String> {
    use regex::Regex;
    use std::sync::LazyLock;
    // Aggregate counts alone do not identify the failure that produced them.
    static AGGREGATE: LazyLock<Regex> = LazyLock::new(|| {
        let count = r"\d+\s+(?:passed|failed|skipped|ignored|errors?|warnings?|measured|total|filtered\s+out)";
        let counts = format!(r"{count}(?:\s*[,;]\s*{count})*");
        let elapsed = r"(?:\s+in\s+\d+(?:\.\d+)?\s*(?:ns|[uµμ]s|ms|s|seconds?))?";
        let field = r"(?:failures?|errors?|skipped)\s*=\s*\d+";
        let aborted = r"(?:error:\s*)?aborting\s+due\s+to\s+\d+\s+previous\s+errors?(?:;\s*\d+\s+warnings?\s+emitted)?";
        Regex::new(&format!(r"(?ix)^(?:{aborted}|{counts}{elapsed}|(?:tests?(?:\s+(?:suites?|run))?|summary)\s*:\s*{counts}{elapsed}|FAILED\s*\({field}(?:,\s*{field})*\)|(?:FAIL|FAILED|FAILURE)[.!]?)$"))
            .expect("static pattern")
    });
    let root_pattern = identity_root_pattern(root);
    let located: std::collections::BTreeSet<usize> =
        crate::located_diagnostics::located_lines(output)
            .into_iter()
            .filter(|site| site.kind == crate::located_diagnostics::Kind::Error)
            .map(|site| site.output_line)
            .collect();
    let lines: std::collections::BTreeSet<String> = output
        .lines()
        .enumerate()
        .filter(|(index, line)| {
            located.contains(index)
                || (crate::failure_evidence::is_failure_line(line)
                    && failed_count(line).is_none()
                    && !AGGREGATE.is_match(line.trim()))
        })
        .map(|(index, line)| {
            // Preserve the parser's error role when locations are stored without their header.
            let line =
                if located.contains(&index) && !crate::failure_evidence::is_failure_line(line) {
                    format!("error: {}", line.trim())
                } else {
                    line.to_string()
                };
            let paths = root_pattern.as_ref().map_or_else(
                || line.clone(),
                |pattern| pattern.replace_all(&line, "${before}<root>/").into_owned(),
            );
            mask_run_values(&paths).trim().to_string()
        })
        .collect();
    (!lines.is_empty()).then(|| lines.into_iter().collect::<Vec<_>>().join("\n"))
}

/// Run values are whole tokens; identifier, parameter and path fragments stay intact.
pub(crate) fn mask_run_values(text: &str) -> String {
    use std::sync::LazyLock;
    static VALUES: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(
            r"\b(?:[0-9]+(?:\.[0-9]+)?\s*(?:ns|[uµμ]s|ms|s|m|h))+\b|\b0[xX][0-9a-fA-F]+\b",
        )
        .expect("static pattern")
    });
    VALUES
        .replace_all(text, |captures: &regex::Captures<'_>| {
            let value = captures.get(0).expect("whole match");
            let prefix = &text[..value.start()];
            let suffix = &text[value.end()..];
            let before = prefix
                .rsplit_once(char::is_whitespace)
                .map_or(prefix, |(_, tail)| tail);
            let after = suffix
                .split_once(char::is_whitespace)
                .map_or(suffix, |(head, _)| head);
            if before.chars().all(|ch| matches!(ch, '(' | '\'' | '"'))
                && after
                    .chars()
                    .all(|ch| matches!(ch, ')' | '\'' | '"' | ',' | ';' | '.'))
            {
                "<value>".to_string()
            } else {
                value.as_str().to_string()
            }
        })
        .into_owned()
}

/// Mask only a complete repository prefix, retaining the remaining path.
pub(crate) fn identity_root_pattern(root: &std::path::Path) -> Option<regex::Regex> {
    if root.as_os_str().is_empty() {
        return None;
    }
    let mut roots = vec![root.to_path_buf()];
    if let Ok(canonical) = std::fs::canonicalize(root)
        && !roots.contains(&canonical)
    {
        roots.push(canonical);
    }
    let mut spellings = std::collections::BTreeSet::new();
    for root in roots {
        let raw = root.to_string_lossy();
        let portable = archon_write_plan::lexical_path::portable(&raw);
        for spelling in [raw.as_ref(), portable.as_str()] {
            spellings.insert(regex::escape(spelling.trim_end_matches(['/', '\\'])));
        }
    }
    let prefixes = spellings.into_iter().collect::<Vec<_>>().join("|");
    Some(
        regex::Regex::new(&format!(r#"(?P<before>^|[\s"'(=:])(?:{prefixes})[/\\]"#))
            .expect("escaped root pattern"),
    )
}

/// Every observed red marker must be present in the recorded base set.
pub(crate) fn identity_subset(base: Option<&str>, tip: Option<&str>) -> bool {
    base.zip(tip).is_some_and(|(base, tip)| {
        !tip.trim().is_empty() && tip.lines().all(|line| base.lines().any(|old| old == line))
    })
}

/// Unlocated file tokens count only when the declaration names that file.
pub(crate) fn diagnostic_files_for(
    output: &str,
    root: &std::path::Path,
    command: &str,
) -> Vec<String> {
    let mut files = diagnostic_files(output, root);
    if files.is_empty() {
        for line in output.lines().map(str::trim) {
            if command
                .split_whitespace()
                .any(|word| word.trim_matches(['\'', '"']) == line)
                && let Some(path) = relative_to(line, &[root.to_path_buf()])
                && root.join(&path).is_file()
            {
                files.push(path);
            }
        }
    }
    files.sort();
    files.dedup();
    files
}
