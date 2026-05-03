//! Phase 4 advisory quality gates for specialist outputs.
//!
//! All gates are advisory (log warnings); Phase 5 will harden them into
//! blocking enforcement.

/// Result of running a quality gate check.
#[derive(Debug, Clone)]
pub struct QualityCheck {
    pub passed: bool,
    pub gate_name: &'static str,
    pub detail: String,
}

/// Check that a specialist's output is non-empty.
pub fn check_non_empty(agent_key: &str, output: &str) -> QualityCheck {
    let passed = !output.trim().is_empty();
    QualityCheck {
        passed,
        gate_name: "non-empty-output",
        detail: if passed {
            format!("specialist '{agent_key}' produced non-empty output")
        } else {
            format!("specialist '{agent_key}' produced empty output")
        },
    }
}

/// Check that a specialist's output parses as valid JSON (if it claims to be JSON).
///
/// Heuristic: output that starts with `{` or `[` after trimming whitespace
/// is expected to be valid JSON.
pub fn check_json_parseable(agent_key: &str, output: &str) -> QualityCheck {
    let trimmed = output.trim();
    let looks_like_json = trimmed.starts_with('{') || trimmed.starts_with('[');
    if !looks_like_json {
        return QualityCheck {
            passed: true,
            gate_name: "json-parseable",
            detail: format!("specialist '{agent_key}' output is not JSON-shaped; skipping"),
        };
    }
    let passed = serde_json::from_str::<serde_json::Value>(trimmed).is_ok();
    QualityCheck {
        passed,
        gate_name: "json-parseable",
        detail: if passed {
            format!("specialist '{agent_key}' output is valid JSON")
        } else {
            format!("specialist '{agent_key}' output looks like JSON but fails to parse")
        },
    }
}

/// Check that a specialist output contains at least one citation marker.
///
/// Citation markers: `[^`, `[citation`, `source:`, `reference:` (case-insensitive prefix).
pub fn check_citation_count(agent_key: &str, output: &str) -> QualityCheck {
    let lower = output.to_lowercase();
    let has_doi = lower.contains("doi:") || lower.contains("doi ");
    let has_url = lower.contains("http://") || lower.contains("https://");
    let has_citation = lower.contains("[^") || lower.contains("[citation") || lower.contains("source:") || lower.contains("reference:");
    let passed = has_citation || has_doi || has_url;
    QualityCheck {
        passed,
        gate_name: "citation-count",
        detail: if passed {
            format!("specialist '{agent_key}' output contains citation evidence")
        } else {
            format!("specialist '{agent_key}' output has no detectable citations")
        },
    }
}

/// Run all Phase 4 advisory quality gates against a specialist output.
///
/// Returns warnings for each failed gate. Phase 5 will make failures blocking.
pub fn run_advisory_gates(agent_key: &str, output: &str) -> Vec<QualityCheck> {
    vec![
        check_non_empty(agent_key, output),
        check_json_parseable(agent_key, output),
        check_citation_count(agent_key, output),
    ]
}

/// Log warnings for any failed quality gates.
pub fn log_failed_gates(checks: &[QualityCheck]) {
    for check in checks {
        if !check.passed {
            tracing::warn!(
                "gametheory quality gate [{}] FAILED: {}",
                check.gate_name,
                check.detail
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_non_empty_detects_empty_output() {
        let check = check_non_empty("gt-test", "");
        assert!(!check.passed);

        let check2 = check_non_empty("gt-test", "valid output");
        assert!(check2.passed);
    }

    #[test]
    fn test_json_parseable_with_valid_json() {
        let check = check_json_parseable("gt-test", r#"{"key": "value"}"#);
        assert!(check.passed);

        let check2 = check_json_parseable("gt-test", r#"not json at all"#);
        assert!(check2.passed); // doesn't look like JSON, so skip
    }

    #[test]
    fn test_json_parseable_with_invalid_json() {
        let check = check_json_parseable("gt-test", r#"{"broken": "#);
        assert!(!check.passed);
    }

    #[test]
    fn test_citation_detection() {
        assert!(check_citation_count("gt-test", "see [^1] for details").passed);
        assert!(check_citation_count("gt-test", "doi: 10.1234/foo").passed);
        assert!(check_citation_count("gt-test", "no citations here").passed == false);
    }

    #[test]
    fn test_run_advisory_gates_returns_three_checks() {
        let checks = run_advisory_gates("gt-test", "some output with http://example.com");
        assert_eq!(checks.len(), 3);
    }
}
