use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::fixtures::LabeledTurnFixture;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FixtureAuditFinding {
    pub fixture_id: String,
    pub field: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FixtureAuditReport {
    pub fixture_count: usize,
    pub finding_count: usize,
    pub findings: Vec<FixtureAuditFinding>,
}

impl FixtureAuditReport {
    pub fn passed(&self) -> bool {
        self.findings.is_empty()
    }
}

pub fn audit_labeled_turns(fixtures: &[LabeledTurnFixture]) -> FixtureAuditReport {
    let mut findings = Vec::new();
    for fixture in fixtures {
        scan_field(
            &mut findings,
            fixture,
            "assistant_text",
            &fixture.assistant_text,
        );
    }
    FixtureAuditReport {
        fixture_count: fixtures.len(),
        finding_count: findings.len(),
        findings,
    }
}

fn scan_field(
    findings: &mut Vec<FixtureAuditFinding>,
    fixture: &LabeledTurnFixture,
    field: &str,
    value: &str,
) {
    for (pattern, reason) in [
        (r"(?i)bearer\s+[a-z0-9._\-]{12,}", "bearer token"),
        (
            r"(?i)(api[_-]?key|token|secret)\s*[:=]\s*[a-z0-9._\-]{8,}",
            "secret-like assignment",
        ),
        (
            r"[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}",
            "email address",
        ),
        (r"/home/[A-Za-z0-9_.-]+/", "unredacted home path"),
        (r"/Users/[A-Za-z0-9_.-]+/", "unredacted macOS home path"),
        (r"\b[A-Za-z0-9+/=_\-]{48,}\b", "high-entropy token"),
    ] {
        if Regex::new(pattern)
            .map(|regex| regex.is_match(value))
            .unwrap_or(false)
        {
            findings.push(FixtureAuditFinding {
                fixture_id: fixture.fixture_id.clone(),
                field: field.to_string(),
                reason: reason.to_string(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_flags_secret_like_fixture_text() {
        let fixtures = vec![LabeledTurnFixture {
            fixture_id: "bad".to_string(),
            assistant_text: "token=abcdefghijklmnopqrstuvwxyz123456".to_string(),
            ..LabeledTurnFixture::default()
        }];
        let report = audit_labeled_turns(&fixtures);
        assert!(!report.passed());
        assert_eq!(report.findings[0].reason, "secret-like assignment");
    }
}
