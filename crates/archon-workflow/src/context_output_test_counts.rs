pub(super) fn reports_zero_test_filter(body: &str, lower: &str) -> bool {
    explicit_zero_test_phrase(lower) || rust_summary_reports_zero_matched(body)
}

const MAX_EVIDENCE_COMMANDS: usize = 3;
const MAX_EVIDENCE_TEXT: usize = 200;

/// Identify WHICH command matched zero tests so the rejection reason carries
/// actionable feedback: bounded repair loops re-prescribe blind when the
/// offending command is dropped from the typed error.
pub(super) fn zero_test_filter_evidence(body: &str) -> Option<String> {
    json_zero_match_commands(body).or_else(|| text_zero_match_evidence(body))
}

fn json_zero_match_commands(body: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(body.trim()).ok()?;
    let mut commands = Vec::new();
    collect_zero_match_commands(&value, &mut commands);
    if commands.is_empty() {
        return None;
    }
    commands.truncate(MAX_EVIDENCE_COMMANDS);
    Some(format!(
        "offending test command(s): {}",
        commands
            .iter()
            .map(|command| format!("`{}`", truncated(command)))
            .collect::<Vec<_>>()
            .join("; ")
    ))
}

fn collect_zero_match_commands(value: &serde_json::Value, commands: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(fields) => {
            let command = fields.get("command").and_then(serde_json::Value::as_str);
            let output = fields
                .get("output_summary")
                .or_else(|| fields.get("output"))
                .and_then(serde_json::Value::as_str);
            if let (Some(command), Some(output)) = (command, output)
                && reports_zero_test_filter(output, &output.to_ascii_lowercase())
                && !commands.iter().any(|known| known == command)
            {
                commands.push(command.to_string());
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

fn text_zero_match_evidence(body: &str) -> Option<String> {
    let lines: Vec<&str> = body.lines().collect();
    let offending = lines.iter().position(|line| {
        RustTestSummary::parse(line).is_some_and(|summary| summary.matched_tests() == 0)
            || explicit_zero_test_phrase(&line.to_ascii_lowercase())
    })?;
    let command = lines[..offending]
        .iter()
        .rev()
        .find(|line| line_names_test_invocation(line));
    let summary_line = truncated(lines[offending].trim());
    Some(match command {
        Some(command) => format!(
            "offending test command: `{}`; output: `{summary_line}`",
            truncated(command.trim())
        ),
        None => format!("offending output: `{summary_line}`"),
    })
}

fn line_names_test_invocation(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    lower.contains("cargo test") || lower.contains("cargo nextest") || lower.contains("go test")
}

fn truncated(text: &str) -> String {
    let mut result: String = text.chars().take(MAX_EVIDENCE_TEXT).collect();
    if result.len() < text.len() {
        result.push('…');
    }
    result
}

fn explicit_zero_test_phrase(lower: &str) -> bool {
    const PHRASES: &[&str] = &[
        "0-test",
        "zero tests",
        "matched zero tests",
        "matches zero tests",
        "running 0 tests",
    ];
    PHRASES.iter().any(|phrase| lower.contains(phrase))
}

fn rust_summary_reports_zero_matched(body: &str) -> bool {
    body.lines()
        .filter_map(RustTestSummary::parse)
        .any(|summary| summary.matched_tests() == 0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RustTestSummary {
    passed: u64,
    failed: u64,
    ignored: u64,
    measured: u64,
}

impl RustTestSummary {
    fn parse(line: &str) -> Option<Self> {
        let lower = line.to_ascii_lowercase();
        if !lower.contains(" passed;") || !lower.contains(" failed") {
            return None;
        }
        Some(Self {
            passed: count_before(&lower, " passed")?,
            failed: count_before(&lower, " failed")?,
            ignored: count_before(&lower, " ignored").unwrap_or(0),
            measured: count_before(&lower, " measured").unwrap_or(0),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_filtered_rust_test_summary_is_not_zero_matched() {
        let body = "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1007 filtered out.";
        assert!(!reports_zero_test_filter(body, &body.to_ascii_lowercase()));
    }

    #[test]
    fn all_filtered_rust_test_summary_is_zero_matched() {
        let body = "test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 1007 filtered out.";
        assert!(reports_zero_test_filter(body, &body.to_ascii_lowercase()));
    }

    #[test]
    fn evidence_names_zero_match_command_from_json_envelope() {
        let body = r#"{
            "commands_run": [
                {"command": "cargo fmt --all -- --check", "output_summary": "clean"},
                {"command": "cargo test -p wrong-crate some_test -- --nocapture",
                 "output_summary": "test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 42 filtered out."}
            ]
        }"#;
        let evidence = zero_test_filter_evidence(body).unwrap();
        assert!(evidence.contains("cargo test -p wrong-crate some_test"));
    }

    #[test]
    fn evidence_pairs_text_summary_with_preceding_command_line() {
        let body = "ran: cargo test some_filter\ntest result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 9 filtered out.";
        let evidence = zero_test_filter_evidence(body).unwrap();
        assert!(evidence.contains("cargo test some_filter"));
        assert!(evidence.contains("0 passed"));
    }

    #[test]
    fn evidence_absent_when_no_zero_match_present() {
        let body = "test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 9 filtered out.";
        assert!(zero_test_filter_evidence(body).is_none());
    }
}
