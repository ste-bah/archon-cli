//! Gate enforcement for the coding pipeline.
//!
//! Contains:
//! - **ForbiddenPatternScanner** (REQ-IMPROVE-005): blocks TODO, stubs, empty bodies
//! - **CompilationGate** (REQ-IMPROVE-006): `cargo build` / `npm run build` must exit 0
//! - **OrphanDetectionGate** (REQ-IMPROVE-008): every new file must be referenced

use std::path::{Path, PathBuf};
use std::time::Duration;

use super::compilation_gate::{CleanupOutcome, CommandExecution, CommandSpec, TreeTermination};

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
        let path = path.replace('\\', "/");

        // Path component checks
        if path.contains("/test/") || path.contains("/tests/") {
            return true;
        }

        // Extract the filename (last component)
        let filename = path.rsplit('/').next().unwrap_or(&path);

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

// Shared gate types (REQ-IMPROVE-006, REQ-IMPROVE-008)

#[path = "gates_commands.rs"]
mod commands;
pub use commands::{GateCommand, Language};

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

// Compilation Gate (REQ-IMPROVE-006)

/// Gate that runs `cargo build` or `npm run build` and requires exit code 0.
pub struct CompilationGate;

const COMPILATION_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const MAX_TIMEOUT_EVIDENCE_BYTES: usize = 1_024;

impl CompilationGate {
    /// Execute the build command. Returns gate result with full compiler output.
    pub async fn run(&self, project_root: &Path, language: Language) -> GateResultRecord {
        let command = language.build_command();
        self.run_command(
            CommandSpec::new(&command.program, command.args, project_root),
            COMPILATION_TIMEOUT,
        )
        .await
    }

    pub(crate) async fn run_command(
        &self,
        command: CommandSpec,
        timeout: Duration,
    ) -> GateResultRecord {
        let command_display = command.display();
        let program_display = command.program_display();
        match super::compilation_gate::execute(command, timeout).await {
            Ok(CommandExecution::Completed(out)) => compilation_result(out),
            Ok(CommandExecution::TimedOut { child, tree }) => {
                compilation_timeout_result(&command_display, timeout, child, tree)
            }
            Err(e) => compilation_spawn_failure_result(&program_display, e),
        }
    }
}

fn compilation_result(out: std::process::Output) -> GateResultRecord {
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let evidence = format!("STDOUT:\n{}\nSTDERR:\n{}", stdout, stderr);
    let gate_passed = out.status.success();
    let failures = (!gate_passed)
        .then(|| GateFailure {
            description: "Compilation failed".into(),
            file: None,
            details: evidence.clone(),
        })
        .into_iter()
        .collect();

    GateResultRecord {
        gate_name: "compilation".into(),
        gate_passed,
        evidence,
        failures,
        timestamp: now_iso8601(),
    }
}

fn compilation_timeout_result(
    command: &str,
    timeout: Duration,
    cleanup: CleanupOutcome,
    tree: TreeTermination,
) -> GateResultRecord {
    let cleanup_evidence = format!("{}; {}", cleanup.evidence(), tree.evidence());
    let prefix = "Compilation timeout: command `";
    let suffix = format!(
        "` exceeded limit {}; {cleanup_evidence}.",
        format_timeout(timeout),
    );
    let command = truncate_for_timeout_evidence(
        command,
        MAX_TIMEOUT_EVIDENCE_BYTES.saturating_sub(prefix.len() + suffix.len()),
    );
    let evidence = format!("{prefix}{command}{suffix}");
    GateResultRecord {
        gate_name: "compilation".into(),
        gate_passed: false,
        failures: vec![GateFailure {
            description: "Compilation timed out".into(),
            file: None,
            details: evidence.clone(),
        }],
        evidence,
        timestamp: now_iso8601(),
    }
}

fn compilation_spawn_failure_result(command: &str, error: std::io::Error) -> GateResultRecord {
    GateResultRecord {
        gate_name: "compilation".into(),
        gate_passed: false,
        evidence: format!("Failed to execute {command}: {error}"),
        failures: vec![GateFailure {
            description: "Build command failed to execute".into(),
            file: None,
            details: error.to_string(),
        }],
        timestamp: now_iso8601(),
    }
}

fn format_timeout(timeout: Duration) -> String {
    if timeout.as_secs() > 0 && timeout.subsec_millis() == 0 {
        format!("{}s", timeout.as_secs())
    } else {
        format!("{}ms", timeout.as_millis())
    }
}

fn truncate_for_timeout_evidence(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_owned();
    }

    let end = value
        .char_indices()
        .map(|(index, _)| index)
        .take_while(|index| *index <= max_bytes.saturating_sub(3))
        .last()
        .unwrap_or(0);
    format!("{}...", &value[..end])
}

#[cfg(test)]
#[path = "gates_command_tests.rs"]
mod command_tests;
#[cfg(test)]
#[path = "gates_timeout_tests.rs"]
mod tests;

/// Gate that checks every new file is referenced by at least one other file.
pub struct OrphanDetectionGate;

impl OrphanDetectionGate {
    /// Scan eligible source files once and report every new file's references.
    pub async fn run(&self, new_files: &[PathBuf], project_root: &Path) -> GateResultRecord {
        super::orphan_gate::orphan_result(new_files, project_root, now_iso8601())
    }
}

// Manual Override (EC-PIPE-010)

/// A manual override for a gate that cannot be automatically verified.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManualOverride {
    pub justification: String,
    pub overridden_by: String,
    pub timestamp: String,
}

// Tests Run Gate (REQ-IMPROVE-019)

/// Gate that executes test suites and requires exit code 0.
/// Evidence: full test runner output with pass/fail counts.
pub struct TestsRunGate;

impl TestsRunGate {
    /// Execute test suites and require exit 0.
    pub async fn run(&self, project_root: &Path, language: Language) -> GateResultRecord {
        let command = language.test_command();
        let cmd = command.program.as_str();
        let args: Vec<_> = command.args.iter().map(String::as_str).collect();
        let output = pipeline_output(cmd, &args, project_root);

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

#[path = "gates_smoke.rs"]
mod smoke;
pub use smoke::E2ESmokeTestGate;

// Persistence

#[path = "gates_store.rs"]
mod store;
pub use store::{load_gate_result, save_gate_result};

// Helpers

#[path = "gates_clock.rs"]
mod clock;
use clock::now_iso8601;

fn pipeline_output(
    program: &str,
    args: &[&str],
    project_root: &Path,
) -> std::io::Result<std::process::Output> {
    let boundary = archon_tools::PipelineBoundary::current(project_root)?;
    let mut command = boundary.command(program).into_std();
    command.args(args).current_dir(project_root);
    archon_shell::read_boundary::prepare_spawn(&command)?;
    let output = command.output();
    boundary.settle()?;
    output
}
