use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::mode::PermissionDecision;

// ---------------------------------------------------------------------------
// Path-based rules (existing)
// ---------------------------------------------------------------------------

/// Check if a path matches any of the given glob patterns.
pub fn path_matches_any(path: &Path, patterns: &[String]) -> bool {
    let path_str = path.to_string_lossy();
    for pattern in patterns {
        if let Ok(glob_pattern) = glob::Pattern::new(pattern)
            && glob_pattern.matches(&path_str)
        {
            return true;
        }
    }
    false
}

/// Determine if a file write should be allowed based on path rules.
///
/// Rules:
/// - If path is in deny_paths: deny
/// - If path is in allow_paths: allow
/// - If path is inside project_dir: allow
/// - Otherwise: risky (needs permission)
pub fn check_write_path(
    path: &Path,
    project_dir: &Path,
    allow_paths: &[String],
    deny_paths: &[String],
) -> PathDecision {
    // Deny list takes priority
    if path_matches_any(path, deny_paths) {
        return PathDecision::Deny;
    }

    // Explicit allow list
    if path_matches_any(path, allow_paths) {
        return PathDecision::Allow;
    }

    // Inside project directory is allowed
    if path.starts_with(project_dir) {
        return PathDecision::Allow;
    }

    // Outside project directory is risky
    PathDecision::NeedsPermission
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathDecision {
    Allow,
    NeedsPermission,
    Deny,
}

// ---------------------------------------------------------------------------
// Fine-grained tool rules
// ---------------------------------------------------------------------------

/// A single tool permission rule with pattern matching.
///
/// Pattern format: `"prefix:*"` where `*` is a wildcard.
/// The pattern is matched against `tool_args` using colon-delimited segments.
/// - `"git:*"` matches any `tool_args` starting with `"git"`.
/// - `"*"` matches anything.
/// - Exact string matches non-wildcard segments.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolRule {
    /// The tool name this rule applies to (e.g. `"Bash"`, `"Write"`).
    pub tool: String,
    /// A colon-delimited pattern matched against tool args.
    pub pattern: String,
}

/// A set of fine-grained permission rules evaluated before mode logic.
///
/// Evaluation order: deny > allow > ask. First match wins within each tier.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RuleSet {
    #[serde(default)]
    pub always_allow: Vec<ToolRule>,
    #[serde(default)]
    pub always_deny: Vec<ToolRule>,
    #[serde(default)]
    pub always_ask: Vec<ToolRule>,
}

impl RuleSet {
    /// An empty rule set with no rules.
    pub fn empty() -> Self {
        Self {
            always_allow: Vec::new(),
            always_deny: Vec::new(),
            always_ask: Vec::new(),
        }
    }

    /// Evaluate rules against a tool invocation.
    ///
    /// Returns `Some(decision)` if a rule matched, `None` if no rule applies
    /// (caller should fall through to mode-based logic).
    ///
    /// Check order: deny first, then allow, then ask.
    pub fn evaluate(&self, tool_name: &str, tool_args: &str) -> Option<PermissionDecision> {
        // Deny rules take absolute precedence
        for rule in &self.always_deny {
            if rule_matches(rule, tool_name, tool_args) {
                return Some(PermissionDecision::Deny(format!(
                    "Blocked by deny rule: tool={}, pattern={}",
                    rule.tool, rule.pattern
                )));
            }
        }

        // Allow rules
        for rule in &self.always_allow {
            if rule_matches(rule, tool_name, tool_args) {
                return Some(PermissionDecision::Allow);
            }
        }

        // Ask rules
        for rule in &self.always_ask {
            if rule_matches(rule, tool_name, tool_args) {
                return Some(PermissionDecision::NeedsPermission(format!(
                    "Rule requires confirmation: tool={}, pattern={}",
                    rule.tool, rule.pattern
                )));
            }
        }

        None
    }
}

/// Check if a rule matches a given tool name and args.
fn rule_matches(rule: &ToolRule, tool_name: &str, tool_args: &str) -> bool {
    // Tool name must match (case-sensitive)
    if rule.tool != tool_name {
        return false;
    }
    pattern_matches(&rule.pattern, tool_args)
}

/// Match a colon-delimited pattern against an input string.
///
/// The pattern and input are split by `":"`.
/// - `"*"` as a segment matches any corresponding segment.
/// - A trailing `"*"` matches any remaining segments.
/// - Non-wildcard segments must match the first whitespace-delimited token of the
///   corresponding input segment exactly.
///
/// Special case: if the pattern has a prefix followed by `":*"`, the input is
/// checked to see if it *starts with* that prefix (space/colon delimited).
pub fn pattern_matches(pattern: &str, input: &str) -> bool {
    // Wildcard matches everything
    if pattern == "*" {
        return true;
    }

    let pat_parts: Vec<&str> = pattern.split(':').collect();
    let input_parts: Vec<&str> = input.split(':').collect();

    // Common case: "git:*" pattern matching against "git status" (no colons in input)
    // In this case, treat the first whitespace-delimited word of input as the
    // first segment.
    if pat_parts.len() == 2 && pat_parts[1] == "*" && input_parts.len() == 1 {
        let first_word = input.split_whitespace().next().unwrap_or("");
        return first_word == pat_parts[0];
    }

    // General segment matching
    for (i, pat_seg) in pat_parts.iter().enumerate() {
        if *pat_seg == "*" {
            // Trailing wildcard matches rest
            return true;
        }
        if i >= input_parts.len() {
            return false;
        }
        let input_token = input_parts[i].split_whitespace().next().unwrap_or("");
        if input_token != *pat_seg {
            return false;
        }
    }

    // If pattern segments exhausted, input may have more — that is fine
    true
}
