//! Gate enforcement for the coding pipeline.
//!
//! Contains:
//! - **ForbiddenPatternScanner** (REQ-IMPROVE-005): blocks TODO, stubs, empty bodies
//! - **CompilationGate** (REQ-IMPROVE-006): `cargo build` / `npm run build` must exit 0
//! - **OrphanDetectionGate** (REQ-IMPROVE-008): every new file must be referenced

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Result;
use regex::Regex;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Result of scanning one or more files for forbidden patterns.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanResult {
    pub matches: Vec<PatternMatch>,
    pub scanned_files: usize,
    pub gate_passed: bool,
}

/// A single forbidden-pattern match within a file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternMatch {
    pub file: String,
    pub line: u32,
    pub pattern_name: String,
    pub matched_text: String,
}

/// Severity level for a forbidden pattern.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum Severity {
    Error,
    Warning,
}

/// A compiled forbidden pattern with metadata.
pub struct ForbiddenPattern {
    pub name: String,
    pub regex: Regex,
    pub severity: Severity,
}

// ---------------------------------------------------------------------------
// Scanner
// ---------------------------------------------------------------------------

/// Scans source code for forbidden patterns that indicate incomplete work.
pub struct ForbiddenPatternScanner {
    patterns: Vec<ForbiddenPattern>,
}

impl ForbiddenPatternScanner {
    /// Create a scanner pre-loaded with all default forbidden patterns.
    pub fn new() -> Self {
        let patterns = vec![
            ForbiddenPattern {
                name: "todo-comment".into(),
                regex: Regex::new(r"(?i)\b(TODO|FIXME|HACK|XXX)\b").unwrap(),
                severity: Severity::Warning,
            },
            ForbiddenPattern {
                name: "todo-macro".into(),
                regex: Regex::new(r"\btodo!\s*\(").unwrap(),
                severity: Severity::Error,
            },
            ForbiddenPattern {
                name: "unimplemented-macro".into(),
                regex: Regex::new(r"\bunimplemented!\s*\(").unwrap(),
                severity: Severity::Error,
            },
            ForbiddenPattern {
                name: "stub-keyword".into(),
                regex: Regex::new(r"(?i)\bstub\b").unwrap(),
                severity: Severity::Warning,
            },
            ForbiddenPattern {
                name: "placeholder-keyword".into(),
                regex: Regex::new(r"(?i)\bplaceholder\b").unwrap(),
                severity: Severity::Warning,
            },
            ForbiddenPattern {
                name: "throw-not-implemented".into(),
                regex: Regex::new(r#"throw\s+new\s+Error\s*\(\s*["']not implemented["']\s*\)"#)
                    .unwrap(),
                severity: Severity::Error,
            },
            ForbiddenPattern {
                name: "empty-fn-body-rust".into(),
                regex: Regex::new(r"\bfn\s+\w+[^{]*\{\s*\}").unwrap(),
                severity: Severity::Error,
            },
            ForbiddenPattern {
                name: "empty-fn-body-ts".into(),
                regex: Regex::new(r"\bfunction\s+\w+\s*\([^)]*\)\s*\{\s*\}").unwrap(),
                severity: Severity::Error,
            },
            ForbiddenPattern {
                name: "python-pass-body".into(),
                regex: Regex::new(r"^\s+pass\s*$").unwrap(),
                severity: Severity::Error,
            },
            ForbiddenPattern {
                name: "allow-dead-code".into(),
                regex: Regex::new(r"#\[allow\(dead_code\)\]").unwrap(),
                severity: Severity::Warning,
            },
        ];

        Self { patterns }
    }

    /// Returns `true` if the given path looks like a test file.
    pub fn is_test_file(path: &str) -> bool {
        // Path component checks
        if path.contains("/test/") || path.contains("/tests/") {
            return true;
        }

        // Extract the filename (last component)
        let filename = path.rsplit('/').next().unwrap_or(path);

        // Suffix patterns
        if filename.ends_with("_test.rs")
            || filename.ends_with("_test.py")
            || filename.ends_with(".test.ts")
            || filename.ends_with(".test.js")
        {
            return true;
        }

        // Prefix pattern
        if filename.starts_with("test_") {
            return true;
        }

        false
    }

    /// Scan a single file's content for forbidden patterns.
    ///
    /// Returns an empty vec if the file is a test file.
    pub fn scan_content(&self, file_path: &str, content: &str) -> Vec<PatternMatch> {
        if Self::is_test_file(file_path) {
            return Vec::new();
        }

        let mut matches = Vec::new();

        for (line_idx, line) in content.lines().enumerate() {
            for pattern in &self.patterns {
                if pattern.regex.is_match(line) {
                    matches.push(PatternMatch {
                        file: file_path.to_string(),
                        line: (line_idx + 1) as u32,
                        pattern_name: pattern.name.clone(),
                        matched_text: line.to_string(),
                    });
                }
            }
        }

        matches
    }

    /// Scan multiple files, skipping test files, and return an aggregate result.
    pub fn scan_files(&self, files: &[(&str, &str)]) -> ScanResult {
        let mut all_matches = Vec::new();
        let mut scanned = 0;

        for &(path, content) in files {
            if Self::is_test_file(path) {
                continue;
            }
            scanned += 1;
            let file_matches = self.scan_content(path, content);
            all_matches.extend(file_matches);
        }

        ScanResult {
            gate_passed: all_matches.is_empty(),
            matches: all_matches,
            scanned_files: scanned,
        }
    }
}

impl Default for ForbiddenPatternScanner {
    fn default() -> Self {
        Self::new()
    }
}

// ===========================================================================
// Shared gate types (REQ-IMPROVE-006, REQ-IMPROVE-008)
// ===========================================================================

/// Language of the project being built.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum Language {
    Rust,
    TypeScript,
}

/// Result of running a pipeline gate. Deterministic — uses tool output only.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateResultRecord {
    pub gate_name: String,
    pub gate_passed: bool,
    pub evidence: String,
    pub failures: Vec<GateFailure>,
    pub timestamp: String,
}

/// A single failure within a gate result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateFailure {
    pub description: String,
    pub file: Option<String>,
    pub details: String,
}

// ===========================================================================
// Compilation Gate (REQ-IMPROVE-006)
// ===========================================================================

/// Gate that runs `cargo build` or `npm run build` and requires exit code 0.
pub struct CompilationGate;

impl CompilationGate {
    /// Execute the build command. Returns gate result with full compiler output.
    pub async fn run(&self, project_root: &Path, language: Language) -> GateResultRecord {
        let (cmd, args) = match language {
            Language::Rust => ("cargo", vec!["build"]),
            Language::TypeScript => ("npm", vec!["run", "build"]),
        };

        let output = Command::new(cmd)
            .args(&args)
            .current_dir(project_root)
            .output();

        match output {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                let stderr = String::from_utf8_lossy(&out.stderr);
                let combined = format!("STDOUT:\n{}\nSTDERR:\n{}", stdout, stderr);
                let passed = out.status.success();

                let failures = if passed {
                    vec![]
                } else {
                    vec![GateFailure {
                        description: "Compilation failed".into(),
                        file: None,
                        details: combined.clone(),
                    }]
                };

                GateResultRecord {
                    gate_name: "compilation".into(),
                    gate_passed: passed,
                    evidence: combined,
                    failures,
                    timestamp: now_iso8601(),
                }
            }
            Err(e) => GateResultRecord {
                gate_name: "compilation".into(),
                gate_passed: false,
                evidence: format!("Failed to execute {}: {}", cmd, e),
                failures: vec![GateFailure {
                    description: "Build command failed to execute".into(),
                    file: None,
                    details: e.to_string(),
                }],
                timestamp: now_iso8601(),
            },
        }
    }
}

// ===========================================================================
// Orphan Detection Gate (REQ-IMPROVE-008)
// ===========================================================================

/// Gate that checks every new file is referenced by at least one other file.
pub struct OrphanDetectionGate;

impl OrphanDetectionGate {
    /// For each new file, search project for references. Returns per-file evidence.
    pub async fn run(&self, new_files: &[PathBuf], project_root: &Path) -> GateResultRecord {
        if new_files.is_empty() {
            return GateResultRecord {
                gate_name: "orphan-detection".into(),
                gate_passed: true,
                evidence: "No new files to check.".into(),
                failures: vec![],
                timestamp: now_iso8601(),
            };
        }

        let mut failures = Vec::new();
        let mut evidence_lines = Vec::new();

        for new_file in new_files {
            let stem = new_file.file_stem().and_then(|s| s.to_str()).unwrap_or("");

            if stem.is_empty() {
                continue;
            }

            // Search for references: mod <stem>, use ...<stem>, import ... <stem>
            let references = find_references(stem, project_root, new_file);

            if references.is_empty() {
                let rel = new_file
                    .strip_prefix(project_root)
                    .unwrap_or(new_file)
                    .display()
                    .to_string();
                failures.push(GateFailure {
                    description: format!("Orphaned file: {}", rel),
                    file: Some(rel.clone()),
                    details: format!(
                        "No mod/use/import references to '{}' found in project",
                        stem
                    ),
                });
                evidence_lines.push(format!("ORPHAN: {} — zero references", rel));
            } else {
                let rel = new_file
                    .strip_prefix(project_root)
                    .unwrap_or(new_file)
                    .display()
                    .to_string();
                let refs_str = references.join(", ");
                evidence_lines.push(format!("OK: {} — referenced by: {}", rel, refs_str));
            }
        }

        GateResultRecord {
            gate_name: "orphan-detection".into(),
            gate_passed: failures.is_empty(),
            evidence: evidence_lines.join("\n"),
            failures,
            timestamp: now_iso8601(),
        }
    }
}

/// Search all source files under `project_root` for references to `stem`.
///
/// Looks for patterns: `mod <stem>`, `use ...<stem>`, `import ... <stem>`,
/// `require('<stem>')`. Skips the file itself.
fn find_references(stem: &str, project_root: &Path, skip_file: &Path) -> Vec<String> {
    let mut references = Vec::new();

    let pattern = Regex::new(&format!(
        r#"(?:mod\s+{s}|use\s+[^;]*\b{s}\b|import\s+.*\b{s}\b|require\s*\(\s*['"].*{s})"#,
        s = regex::escape(stem),
    ))
    .unwrap_or_else(|_| Regex::new("^$").unwrap());

    walk_source_files(project_root, &mut |path| {
        // Skip the file itself
        if path == skip_file {
            return;
        }

        if let Ok(content) = std::fs::read_to_string(path) {
            if pattern.is_match(&content) {
                let rel = path
                    .strip_prefix(project_root)
                    .unwrap_or(path)
                    .display()
                    .to_string();
                references.push(rel);
            }
        }
    });

    references
}

/// Walk source files under a directory, calling `f` for each.
fn walk_source_files(dir: &Path, f: &mut dyn FnMut(&Path)) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            if name.starts_with('.') || name == "target" || name == "node_modules" {
                continue;
            }
            walk_source_files(&path, f);
        } else if path.is_file() {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if matches!(ext, "rs" | "ts" | "tsx" | "js" | "jsx" | "py" | "go") {
                f(&path);
            }
        }
    }
}

// ===========================================================================
// Manual Override (EC-PIPE-010)
// ===========================================================================

/// A manual override for a gate that cannot be automatically verified.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManualOverride {
    pub justification: String,
    pub overridden_by: String,
    pub timestamp: String,
}

// ===========================================================================
// Tests Run Gate (REQ-IMPROVE-019)
// ===========================================================================

/// Gate that executes test suites and requires exit code 0.
/// Evidence: full test runner output with pass/fail counts.
pub struct TestsRunGate;

impl TestsRunGate {
    /// Execute test suites and require exit 0.
    pub async fn run(&self, project_root: &Path, language: Language) -> GateResultRecord {
        let (cmd, args) = match language {
            Language::Rust => ("cargo", vec!["test"]),
            Language::TypeScript => ("npm", vec!["test"]),
        };

        let output = Command::new(cmd)
            .args(&args)
            .current_dir(project_root)
            .output();

        match output {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                let stderr = String::from_utf8_lossy(&out.stderr);
                let combined = format!("STDOUT:\n{}\nSTDERR:\n{}", stdout, stderr);
                let passed = out.status.success();

                let failures = if passed {
                    vec![]
                } else {
                    vec![GateFailure {
                        description: "Test suite failed".into(),
                        file: None,
                        details: combined.clone(),
                    }]
                };

                GateResultRecord {
                    gate_name: "tests-run".into(),
                    gate_passed: passed,
                    evidence: combined,
                    failures,
                    timestamp: now_iso8601(),
                }
            }
            Err(e) => GateResultRecord {
                gate_name: "tests-run".into(),
                gate_passed: false,
                evidence: format!("Failed to execute {}: {}", cmd, e),
                failures: vec![GateFailure {
                    description: "Test command failed to execute".into(),
                    file: None,
                    details: e.to_string(),
                }],
                timestamp: now_iso8601(),
            },
        }
    }
}

// ===========================================================================
// E2E Smoke Test Gate (REQ-IMPROVE-010)
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

        let output = Command::new(parts[0])
            .args(&parts[1..])
            .current_dir(project_root)
            .output();

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
            .any(|p| Regex::new(p).map_or(false, |re| re.is_match(output)));

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

// ===========================================================================
// Persistence
// ===========================================================================

/// Save a gate result to `<session_dir>/gate-results/<gate_name>.json`.
pub fn save_gate_result(result: &GateResultRecord, session_dir: &Path) -> Result<()> {
    let dir = session_dir.join("gate-results");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.json", result.gate_name));
    let json = serde_json::to_string_pretty(result)?;
    std::fs::write(&path, json)?;
    Ok(())
}

/// Load a gate result from `<session_dir>/gate-results/<gate_name>.json`.
pub fn load_gate_result(gate_name: &str, session_dir: &Path) -> Result<GateResultRecord> {
    let path = session_dir
        .join("gate-results")
        .join(format!("{}.json", gate_name));
    let data = std::fs::read_to_string(&path)?;
    let result: GateResultRecord = serde_json::from_str(&data)?;
    Ok(result)
}

// ===========================================================================
// Helpers
// ===========================================================================

fn now_iso8601() -> String {
    let dur = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    // Simple ISO 8601-ish timestamp
    format!("{}Z", dur.as_secs())
}
