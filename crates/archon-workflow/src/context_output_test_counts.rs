pub(super) fn reports_zero_test_filter(body: &str, lower: &str) -> bool {
    explicit_zero_test_phrase(lower) || rust_summary_reports_zero_matched(body)
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
}
