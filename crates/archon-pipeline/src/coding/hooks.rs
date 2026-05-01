//! Pre/PostToolUse hook policies for the coding pipeline.
//!
//! Implements REQ-IMPROVE-011 (PreToolUse) and REQ-IMPROVE-012 (PostToolUse).
//! Enforces phase-based tool restrictions, dangerous command blocking,
//! affected-file scope, and post-execution quality checks.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use regex::Regex;
use serde_json::Value;

use super::gates::ForbiddenPatternScanner;

// ---------------------------------------------------------------------------
// Hook Decision
// ---------------------------------------------------------------------------

/// Result of evaluating a tool-use hook.
#[derive(Debug, Clone)]
pub enum HookDecision {
    /// Tool call is permitted.
    Allow,
    /// Tool call is blocked — must not execute.
    Block { reason: String },
    /// Tool call is permitted, but a warning is injected into agent context.
    Warn { message: String },
}

// ---------------------------------------------------------------------------
// Dangerous command patterns
// ---------------------------------------------------------------------------

const DANGEROUS_PATTERNS: &[&str] = &[
    r"rm\s+-rf",
    r"git\s+push",
    r"git\s+merge",
    r"(?i)DROP\s+TABLE",
    r"git\s+reset\s+--hard",
    r"git\s+checkout\s+--\s+\.",
];

fn compile_dangerous_patterns() -> Vec<Regex> {
    DANGEROUS_PATTERNS
        .iter()
        .filter_map(|p| Regex::new(p).ok())
        .collect()
}

// ---------------------------------------------------------------------------
// Read-only tools (allowed in all phases)
// ---------------------------------------------------------------------------

const READ_ONLY_TOOLS: &[&str] = &[
    "Read",
    "Glob",
    "Grep",
    "WebSearch",
    "WebFetch",
    "LS",
    "Agent",
    "TaskCreate",
    "TaskUpdate",
    "TaskGet",
    "TaskList",
];

fn is_read_only_tool(tool_name: &str) -> bool {
    READ_ONLY_TOOLS.contains(&tool_name)
}

/// Tools that are restricted in read-only phases (1-3).
fn is_write_tool(tool_name: &str) -> bool {
    matches!(tool_name, "Write" | "Edit" | "Bash" | "NotebookEdit")
}

// ---------------------------------------------------------------------------
// PreToolUse Hook
// ---------------------------------------------------------------------------

/// Hook that evaluates tool calls BEFORE execution.
///
/// Enforces:
/// - Phase 1-3: deny Write, Edit, Bash
/// - All phases: deny dangerous Bash commands
/// - Scope: deny/warn on files outside affected_files
pub struct PreToolUseHook {
    current_phase: u8,
    affected_files: Vec<String>,
    dangerous_patterns: Vec<Regex>,
}

impl PreToolUseHook {
    pub fn new(phase: u8, affected_files: Vec<String>) -> Self {
        Self {
            current_phase: phase,
            affected_files,
            dangerous_patterns: compile_dangerous_patterns(),
        }
    }

    /// Evaluate a tool call before execution.
    pub fn evaluate(&self, tool_name: &str, tool_input: &Value) -> HookDecision {
        // 1. Read-only tools are always allowed
        if is_read_only_tool(tool_name) {
            return HookDecision::Allow;
        }

        // 2. Phase 1-3: block all write tools
        if self.current_phase <= 3 && is_write_tool(tool_name) {
            return HookDecision::Block {
                reason: format!(
                    "{} denied in Phase {} (read-only)",
                    tool_name, self.current_phase
                ),
            };
        }

        // 3. Bash: check dangerous command patterns (any phase)
        if tool_name == "Bash"
            && let Some(cmd) = tool_input.get("command").and_then(|v| v.as_str()) {
                for pattern in &self.dangerous_patterns {
                    if pattern.is_match(cmd) {
                        return HookDecision::Block {
                            reason: format!(
                                "Dangerous command blocked: matches pattern '{}'",
                                pattern.as_str()
                            ),
                        };
                    }
                }
            }

        // 4. Write/Edit: check file scope
        if matches!(tool_name, "Write" | "Edit")
            && let Some(file_path) = tool_input.get("file_path").and_then(|v| v.as_str())
                && !self.affected_files.is_empty()
                    && !self.affected_files.iter().any(|af| file_path.contains(af))
                {
                    if self.current_phase <= 3 {
                        return HookDecision::Block {
                            reason: format!("File '{}' is outside affected_files scope", file_path),
                        };
                    } else {
                        return HookDecision::Warn {
                            message: format!(
                                "WARNING: Writing to '{}' which is outside the contract's affected_files list",
                                file_path
                            ),
                        };
                    }
                }

        HookDecision::Allow
    }
}

// ---------------------------------------------------------------------------
// PostToolUse Hook
// ---------------------------------------------------------------------------

/// Hook that evaluates tool calls AFTER execution.
///
/// Provides:
/// - Forbidden pattern scanning on Write/Edit content
/// - Periodic `cargo check` after every 5th Edit in Phase 4+
/// - Orphan detection after Write
pub struct PostToolUseHook {
    _project_root: PathBuf,
    edit_counter: AtomicU32,
    scanner: ForbiddenPatternScanner,
}

impl PostToolUseHook {
    pub fn new(project_root: PathBuf) -> Self {
        Self {
            _project_root: project_root,
            edit_counter: AtomicU32::new(0),
            scanner: ForbiddenPatternScanner::new(),
        }
    }

    /// Check content for forbidden patterns. Returns warning message if found.
    pub fn check_forbidden_patterns(&self, content: &str) -> Option<String> {
        let matches = self.scanner.scan_content("<inline>", content);
        if matches.is_empty() {
            None
        } else {
            let details: Vec<String> = matches
                .iter()
                .map(|m| {
                    format!(
                        "  - line {}: {} ({})",
                        m.line,
                        m.pattern_name,
                        m.matched_text.trim()
                    )
                })
                .collect();
            Some(format!(
                "WARNING: Forbidden patterns detected:\n{}",
                details.join("\n")
            ))
        }
    }

    /// Increment the edit counter and return the new value.
    pub fn increment_edit_counter(&self) -> u32 {
        self.edit_counter.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// Check if a compilation check should run (every 5th edit in Phase 4+).
    pub fn should_run_compilation_check(&self, phase: u8) -> bool {
        if phase < 4 {
            return false;
        }
        let count = self.edit_counter.load(Ordering::Relaxed);
        count > 0 && count.is_multiple_of(5)
    }

    /// Reset the edit counter (call when starting a new agent).
    pub fn reset_edit_counter(&self) {
        self.edit_counter.store(0, Ordering::Relaxed);
    }
}
