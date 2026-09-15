//! Zero-match test detection: a filtered test command that executed nothing
//! is never verification evidence, but only a RUNNER's own summary proves
//! that. Prose is never scanned (Issue-17: "coverage_tests compile zero
//! tests" in an evidence line failed a good branch), and invocations that
//! cannot run tests (`--list`, `--no-run`, builds, lints) are never candidates.

#[cfg(test)]
#[path = "context_output_test_counts_tests.rs"]
mod tests;

const MAX_EVIDENCE_COMMANDS: usize = 3;
const MAX_EVIDENCE_TEXT: usize = 200;

/// A command whose runner output says no test ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ZeroMatchCommand {
    pub command: String,
    /// The envelope presents the command as passing evidence: an explicit
    /// passing status, exit 0, or no failure marker at all (fail closed).
    pub presented_as_passing: bool,
}

/// Every command in `body` whose runner output reports zero matched tests.
/// JSON bodies are walked entry by entry; plain text pairs a runner summary
/// line with the nearest preceding invocation line.
pub(super) fn zero_match_commands(body: &str) -> Vec<ZeroMatchCommand> {
    match serde_json::from_str::<serde_json::Value>(body.trim()) {
        Ok(value) => {
            let mut commands = Vec::new();
            collect_zero_match_commands(&value, &mut commands);
            commands
        }
        Err(_) => text_zero_match_commands(body),
    }
}

/// The rejection reason's evidence clause for the fatal commands, bounded.
pub(super) fn zero_match_evidence(commands: &[ZeroMatchCommand]) -> String {
    format!(
        "offending test command(s): {}",
        commands
            .iter()
            .take(MAX_EVIDENCE_COMMANDS)
            .map(|entry| format!("`{}`", truncated(&entry.command)))
            .collect::<Vec<_>>()
            .join("; ")
    )
}

const OUTPUT_FIELDS: &[&str] = &["output_summary", "output", "stdout", "result"];
const STATUS_FIELDS: &[&str] = &["status", "outcome"];
const EXIT_FIELDS: &[&str] = &["exit_code", "exit_status", "exit"];
const PASSING_STATUSES: &[&str] = &[
    "succeeded",
    "success",
    "ok",
    "passed",
    "pass",
    "complete",
    "completed",
    "done",
];

fn collect_zero_match_commands(value: &serde_json::Value, commands: &mut Vec<ZeroMatchCommand>) {
    match value {
        serde_json::Value::Object(fields) => {
            if let Some(command) = fields.get("command").and_then(serde_json::Value::as_str)
                && !command_is_non_run_invocation(command)
                && OUTPUT_FIELDS
                    .iter()
                    .filter_map(|field| fields.get(*field).and_then(serde_json::Value::as_str))
                    .any(output_reports_zero_matched)
                && !commands.iter().any(|known| known.command == command)
            {
                commands.push(ZeroMatchCommand {
                    command: command.to_string(),
                    presented_as_passing: entry_presented_as_passing(fields),
                });
            }
            for nested in fields.values() {
                collect_zero_match_commands(nested, commands);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_zero_match_commands(item, commands);
            }
        }
        _ => {}
    }
}

fn entry_presented_as_passing(fields: &serde_json::Map<String, serde_json::Value>) -> bool {
    if let Some(status) = STATUS_FIELDS
        .iter()
        .find_map(|field| fields.get(*field).and_then(serde_json::Value::as_str))
    {
        return PASSING_STATUSES.contains(&status.trim().to_ascii_lowercase().as_str());
    }
    if let Some(exit) = EXIT_FIELDS
        .iter()
        .find_map(|field| fields.get(*field).and_then(serde_json::Value::as_i64))
    {
        return exit == 0;
    }
    true
}

fn text_zero_match_commands(body: &str) -> Vec<ZeroMatchCommand> {
    let lines: Vec<&str> = body.lines().collect();
    let mut commands = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if !line_reports_zero_matched(line) {
            continue;
        }
        let invocation = lines[..index]
            .iter()
            .rev()
            .find(|line| line_names_test_invocation(line))
            .map(|line| line.trim());
        if invocation.is_some_and(command_is_non_run_invocation) {
            continue;
        }
        let command = match invocation {
            Some(invocation) => format!("{invocation}; output: {}", line.trim()),
            None => format!("<unnamed>; output: {}", line.trim()),
        };
        if !commands
            .iter()
            .any(|known: &ZeroMatchCommand| known.command == command)
        {
            commands.push(ZeroMatchCommand {
                command,
                presented_as_passing: true,
            });
        }
    }
    commands
}

fn line_names_test_invocation(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    [
        "cargo test",
        "cargo nextest",
        "go test",
        "pytest",
        "jest",
        "mocha",
    ]
    .iter()
    .any(|runner| lower.contains(runner))
}

fn truncated(text: &str) -> String {
    let mut result: String = text.chars().take(MAX_EVIDENCE_TEXT).collect();
    if result.len() < text.len() {
        result.push('…');
    }
    result
}

/// Flags that make an invocation enumerate, compile or describe rather than run.
const NON_RUN_FLAGS: &[&str] = &["--list", "--no-run", "--help", "--version", "--dry-run"];
const CARGO_NON_RUN_SUBCOMMANDS: &[&str] = &["check", "build", "clippy", "fmt", "doc"];
const GO_NON_RUN_SUBCOMMANDS: &[&str] = &["build", "vet", "fmt"];

/// True when `command` cannot have run tests: it carries a listing, no-run,
/// help, version or dry-run flag, or every shell segment is a build or lint.
pub(super) fn command_is_non_run_invocation(command: &str) -> bool {
    let lower = command.to_ascii_lowercase();
    if lower
        .split_whitespace()
        .any(|token| NON_RUN_FLAGS.contains(&token))
    {
        return true;
    }
    let mut segments = lower
        .split(['|', ';', '\n'])
        .flat_map(|segment| segment.split("&&"))
        .filter(|segment| !segment.trim().is_empty())
        .peekable();
    segments.peek().is_some() && segments.all(segment_is_build_or_lint)
}

fn segment_is_build_or_lint(segment: &str) -> bool {
    let tokens: Vec<&str> = segment.split_whitespace().collect();
    tokens.windows(2).any(|pair| {
        (pair[0] == "cargo" && CARGO_NON_RUN_SUBCOMMANDS.contains(&pair[1]))
            || (pair[0] == "go" && GO_NON_RUN_SUBCOMMANDS.contains(&pair[1]))
    })
}

/// Command text as compared against declared focused tests: whitespace
/// collapsed, the same normalisation the read guard applies.
fn normalise_command(command: &str) -> String {
    command.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Shortest abbreviated command allowed to match a longer declared one.
const MIN_ABBREVIATED_COMMAND_CHARS: usize = 12;

/// True when `command` is one of the task's declared focused tests: the read
/// guard's containment rule (the declared text appears in the command), or an
/// agent's abbreviation of a longer declared command.
pub(super) fn command_matches_declared(command: &str, declared: &[String]) -> bool {
    let command = normalise_command(command);
    if command.is_empty() {
        return false;
    }
    declared
        .iter()
        .map(|declared| normalise_command(declared))
        .filter(|declared| !declared.is_empty())
        .any(|declared| {
            command.contains(&declared)
                || (command.chars().count() >= MIN_ABBREVIATED_COMMAND_CHARS
                    && command.contains(' ')
                    && declared.contains(&command))
        })
}

/// True when `output` carries a runner summary line saying no test ran.
pub(super) fn output_reports_zero_matched(output: &str) -> bool {
    output.lines().any(line_reports_zero_matched)
}

/// Runner phrases that open a line (after whitespace and a pytest `=` banner).
const LINE_START_PHRASES: &[&str] = &[
    "no tests found",                    // jest
    "no tests ran",                      // pytest
    "collected 0 items",                 // pytest
    "starting 0 tests",                  // nextest
    "testing: warning: no tests to run", // go
    "0 passing",                         // mocha
];

/// Runner phrases distinctive enough to accept anywhere in a line.
const INLINE_PHRASES: &[&str] = &[
    "running 0 tests",   // cargo test
    "[no tests to run]", // go test package line
    "collected 0 items", // pytest
    "no tests ran in",   // pytest
];

fn line_reports_zero_matched(line: &str) -> bool {
    let lower = line.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return false;
    }
    if RustTestSummary::parse(&lower).is_some_and(|summary| summary.matched_tests() == 0) {
        return true;
    }
    let anchored = lower.trim_start_matches(|ch: char| ch == '=' || ch.is_whitespace());
    LINE_START_PHRASES
        .iter()
        .any(|phrase| anchored.starts_with(phrase))
        || (anchored.starts_with("summary") && anchored.contains(" 0 tests run"))
        || INLINE_PHRASES.iter().any(|phrase| lower.contains(phrase))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RustTestSummary {
    passed: u64,
    failed: u64,
    ignored: u64,
    measured: u64,
}

impl RustTestSummary {
    fn parse(lower: &str) -> Option<Self> {
        if !lower.contains(" passed;") || !lower.contains(" failed") {
            return None;
        }
        Some(Self {
            passed: count_before(lower, " passed")?,
            failed: count_before(lower, " failed")?,
            ignored: count_before(lower, " ignored").unwrap_or(0),
            measured: count_before(lower, " measured").unwrap_or(0),
        })
    }

    fn matched_tests(self) -> u64 {
        self.passed + self.failed + self.ignored + self.measured
    }
}

fn count_before(line: &str, label: &str) -> Option<u64> {
    let before = line.get(..line.find(label)?)?;
    before
        .split(|ch: char| !ch.is_ascii_digit())
        .rev()
        .find(|part| !part.is_empty())
        .and_then(|part| part.parse().ok())
}
