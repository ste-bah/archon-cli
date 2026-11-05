use super::*;

// ===========================================================================
// Smoke command gate
// ===========================================================================

/// Final gate — executes the binary and exercises the new feature.
/// Includes fraud detection to reject test-only evidence.
pub struct E2ESmokeTestGate;

impl E2ESmokeTestGate {
    /// Execute the smoke command and validate the output.
    pub async fn run(
        &self,
        project_root: &Path,
        smoke_command: &str,
        manual_override: Option<ManualOverride>,
    ) -> GateResultRecord {
        // 1. If manual_override is Some, return passed with justification
        if let Some(override_info) = manual_override {
            return GateResultRecord {
                gate_name: "e2e-smoke".into(),
                gate_passed: true,
                evidence: format!(
                    "MANUAL OVERRIDE: {} (by: {}, at: {})",
                    override_info.justification,
                    override_info.overridden_by,
                    override_info.timestamp,
                ),
                failures: vec![],
                timestamp: now_iso8601(),
            };
        }

        // 2. Parse and execute smoke_command
        let parts: Vec<&str> = smoke_command.split_whitespace().collect();
        if parts.is_empty() {
            return GateResultRecord {
                gate_name: "e2e-smoke".into(),
                gate_passed: false,
                evidence: "Empty smoke command".into(),
                failures: vec![GateFailure {
                    description: "No smoke command provided".into(),
                    file: None,
                    details: String::new(),
                }],
                timestamp: now_iso8601(),
            };
        }

        let output = pipeline_output(parts[0], &parts[1..], project_root);

        match output {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                let stderr = String::from_utf8_lossy(&out.stderr);
                let combined = format!("STDOUT:\n{}\nSTDERR:\n{}", stdout, stderr);

                if !out.status.success() {
                    return GateResultRecord {
                        gate_name: "e2e-smoke".into(),
                        gate_passed: false,
                        evidence: combined.clone(),
                        failures: vec![GateFailure {
                            description: "Smoke command exited non-zero".into(),
                            file: None,
                            details: combined,
                        }],
                        timestamp: now_iso8601(),
                    };
                }

                // 3. Fraud detection
                if Self::is_test_only_evidence(&combined) {
                    return GateResultRecord {
                        gate_name: "e2e-smoke".into(),
                        gate_passed: false,
                        evidence: combined.clone(),
                        failures: vec![GateFailure {
                            description: "FRAUD DETECTED: Evidence appears to be test output only, not actual feature invocation".into(),
                            file: None,
                            details: "E2E smoke test requires actual feature execution output, not test runner output".into(),
                        }],
                        timestamp: now_iso8601(),
                    };
                }

                GateResultRecord {
                    gate_name: "e2e-smoke".into(),
                    gate_passed: true,
                    evidence: combined,
                    failures: vec![],
                    timestamp: now_iso8601(),
                }
            }
            Err(e) => GateResultRecord {
                gate_name: "e2e-smoke".into(),
                gate_passed: false,
                evidence: format!("Failed to execute smoke command: {}", e),
                failures: vec![GateFailure {
                    description: "Smoke command failed to execute".into(),
                    file: None,
                    details: e.to_string(),
                }],
                timestamp: now_iso8601(),
            },
        }
    }

    /// Fraud detection: returns true if evidence looks like test-only output.
    pub fn is_test_only_evidence(output: &str) -> bool {
        let test_patterns = [
            r"test result:\s*ok",
            r"\d+\s+passed,?\s*\d*\s*failed",
            r"Tests:\s*\d+\s+passed",
            r"test result:\s*FAILED",
            r"running\s+\d+\s+tests?",
            r"tests?\s+passed",
        ];

        // Check if ANY test pattern matches
        let has_test_pattern = test_patterns
            .iter()
            .any(|p| Regex::new(p).is_ok_and(|re| re.is_match(output)));

        if !has_test_pattern {
            return false;
        }

        // Check for feature-specific output indicators
        let feature_indicators = [
            "HTTP/",
            "200 OK",
            "201 Created",
            "compiled",
            "Compiled",
            "indexed",
            "Indexed",
            "processed",
            "Processed",
            "created",
            "Created",
            "started",
            "Started",
            "listening on",
            "Listening on",
            "\"status\"",
            "\"result\"",
        ];

        let has_feature_output = feature_indicators.iter().any(|ind| output.contains(ind));

        // It's test-only if it has test patterns but no feature indicators
        !has_feature_output
    }
}
