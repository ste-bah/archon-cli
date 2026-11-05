//! Machine-readable test records and summary-only cardinality evidence.
use super::runner_test_ids::Status;

pub(super) fn ids(output: &str, status: Status) -> Vec<String> {
    let mut ids = Vec::new();
    for line in output.lines() {
        if let Ok(record) = serde_json::from_str::<serde_json::Value>(line) {
            let action = record["status"]
                .as_str()
                .or_else(|| record["Action"].as_str());
            let id = record["test_id"]
                .as_str()
                .or_else(|| record["Test"].as_str());
            let matches = match status {
                Status::Passed => matches!(action, Some("passed" | "pass")),
                Status::Failed => matches!(action, Some("failed" | "fail")),
                Status::Skipped => matches!(action, Some("skipped" | "skip")),
            };
            if matches && let Some(id) = id {
                let package = record["Package"].as_str().unwrap_or("");
                ids.push(format!("{package}::{id}"));
            }
        }
    }
    ids.sort();
    ids.dedup();
    ids
}

pub(crate) fn passed_count(output: &str) -> Option<usize> {
    let mut count: Option<usize> = None;
    for line in output.lines().map(str::trim) {
        // pytest/Mocha summaries, including a single result ("1 passed").
        let text = line.trim_matches(['=', ' ']);
        let words: Vec<_> = text.split_whitespace().collect();
        if words
            .first()
            .is_some_and(|word| word.parse::<usize>().is_ok())
        {
            for pair in words.windows(2) {
                if matches!(pair[1].trim_end_matches(','), "passed" | "passing") {
                    count = pair[0].parse().ok();
                }
            }
        }
        // Field-labelled summaries used by managed-runtime runners.
        if let Some((_, passed)) = line.split_once("Passed:") {
            count = passed
                .split_whitespace()
                .next()
                .and_then(|word| word.trim_end_matches(',').parse().ok());
        }
        if let Some(rest) = line.strip_prefix("Ran ") {
            let mut fields = rest.split_whitespace();
            if let (Some(total), Some("test" | "tests")) = (fields.next(), fields.next()) {
                count = total.parse().ok();
            }
        }
        if line.starts_with("OK (") || line.starts_with("FAILED (") {
            let excluded: usize = line
                .split_once('(')
                .into_iter()
                .flat_map(|(_, rest)| rest.trim_end_matches(')').split(','))
                .filter_map(|field| {
                    let (key, value) = field.trim().split_once('=')?;
                    matches!(key, "failures" | "errors" | "skipped")
                        .then(|| value.parse::<usize>().ok())
                        .flatten()
                })
                .sum();
            count = count.map(|total| total.saturating_sub(excluded));
        }
        // Common JVM runner summaries expose cardinality even without ids.
        if let Some(rest) = line.split("Tests run: ").nth(1) {
            let fields: Vec<_> = rest.split(',').collect();
            let total = fields
                .first()
                .and_then(|field| field.trim().parse::<usize>().ok());
            let excluded: usize = fields
                .iter()
                .skip(1)
                .filter_map(|field| {
                    let (key, value) = field.trim().split_once(':')?;
                    matches!(key, "Failures" | "Errors" | "Skipped")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
                .sum();
            count = total.map(|total| total.saturating_sub(excluded));
        }
        if let Some((total, _)) = line.split_once(" tests completed") {
            count = total.trim().parse().ok();
        }
        // Jest summary, TAP/Node summary.
        for prefix in ["Tests:", "# pass ", "# tests "] {
            if let Some(rest) = line.strip_prefix(prefix) {
                if prefix == "Tests:" {
                    count = rest
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .windows(2)
                        .find(|pair| matches!(pair[1].trim_end_matches(','), "passed" | "passing"))
                        .and_then(|pair| pair[0].parse().ok());
                } else if prefix == "# pass " {
                    count = rest.trim().parse().ok();
                }
            }
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quiet_suites_cannot_lose_passing_tests_unnoticed() {
        for (before, after) in [
            ("2 passed in 0.1s", "1 passed in 0.1s"),
            ("2 passing (2ms)", "1 passing (2ms)"),
            (
                "Passed! - Failed: 0, Passed: 2, Skipped: 0",
                "Passed! - Failed: 0, Passed: 1, Skipped: 0",
            ),
            ("Ran 2 tests in 0.1s\nOK", "Ran 1 test in 0.1s\nOK"),
            (
                "Ran 2 tests in 0.1s\nOK",
                "Ran 2 tests in 0.1s\nOK (skipped=1)",
            ),
            ("Tests: 2 passed, 2 total", "Tests: 1 passed, 1 total"),
            ("# pass 2", "# pass 1"),
            (
                "Tests run: 2, Failures: 0, Errors: 0, Skipped: 0",
                "Tests run: 1, Failures: 0, Errors: 0, Skipped: 0",
            ),
        ] {
            assert_eq!(passed_count(before), Some(2));
            assert_eq!(passed_count(after), Some(1));
            assert!(ids(before, Status::Passed).is_empty());
        }
    }
    #[test]
    fn interchange_and_structured_runner_records_preserve_identity() {
        assert_eq!(
            ids(
                r#"{"test_id":"example","status":"skipped"}"#,
                Status::Skipped
            ),
            ["::example"]
        );
        assert_eq!(
            ids(
                r#"{"Test":"TestExample","Action":"pass","Package":"example/pkg"}"#,
                Status::Passed
            ),
            ["example/pkg::TestExample"]
        );
    }
}
