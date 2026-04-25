use crate::mode::{PermissionDecision, PermissionMode};
use crate::rules::RuleSet;

/// Tools allowed in Plan mode (read-only + navigation).
const PLAN_MODE_WHITELIST: &[&str] = &[
    "Read",
    "Glob",
    "Grep",
    "AskUserQuestion",
    "EnterPlanMode",
    "ToolSearch",
];

/// Tools auto-allowed in AcceptEdits mode (file operations + search).
const ACCEPT_EDITS_WHITELIST: &[&str] = &["Read", "Write", "Edit", "Glob", "Grep"];

/// Tools that are safe (read-only) and auto-allowed in Default mode.
const DEFAULT_SAFE_TOOLS: &[&str] = &[
    "Read",
    "Glob",
    "Grep",
    "ToolSearch",
    "AskUserQuestion",
    "EnterPlanMode",
];

/// Permission checker that gates tool execution based on the current mode
/// and fine-grained rule sets.
pub struct PermissionChecker {
    mode: PermissionMode,
    rules: RuleSet,
}

impl PermissionChecker {
    pub fn new(mode: PermissionMode, rules: RuleSet) -> Self {
        Self { mode, rules }
    }

    pub fn mode(&self) -> PermissionMode {
        self.mode
    }

    /// Check if a tool execution should proceed.
    ///
    /// Evaluation order:
    /// 1. Fine-grained rules (deny > allow > ask).
    /// 2. If BypassPermissions, rules still apply but mode allows everything else.
    /// 3. Mode-specific logic.
    pub fn check(&self, tool_name: &str, description: &str, tool_args: &str) -> PermissionDecision {
        // Step 1: Evaluate fine-grained rules (unless BypassPermissions which
        // skips everything — but deny rules still apply even there).
        if let Some(decision) = self.rules.evaluate(tool_name, tool_args) {
            // Deny rules are absolute — they block in every mode including BypassPermissions.
            if matches!(decision, PermissionDecision::Deny(_)) {
                return decision;
            }
            // For non-deny rule decisions, BypassPermissions overrides to Allow.
            if self.mode == PermissionMode::BypassPermissions {
                return PermissionDecision::Allow;
            }
            return decision;
        }

        // Step 2: No rule matched — apply mode logic.
        match self.mode {
            PermissionMode::BypassPermissions => PermissionDecision::Allow,

            PermissionMode::DontAsk => PermissionDecision::Allow,

            PermissionMode::Bubble => {
                // Bubble: delegate to sandbox. Without a concrete sandbox
                // backend in the checker crate (sandbox lives in
                // archon-tui), treat Bubble like Default at the checker
                // layer — user confirmation is required. The tool dispatch
                // layer (which has access to `archon_tui::sandbox`) is
                // responsible for pre-filtering via SandboxGuard BEFORE
                // the permission check fires, so tools that pass the
                // sandbox gate never reach this arm.
                PermissionDecision::NeedsPermission(format!(
                    "Bubble sandbox: user confirmation required for {tool_name}"
                ))
            }

            PermissionMode::Plan => {
                if PLAN_MODE_WHITELIST.contains(&tool_name) {
                    PermissionDecision::Allow
                } else {
                    PermissionDecision::Deny(format!(
                        "Plan mode: tool '{}' is not allowed (read-only mode)",
                        tool_name
                    ))
                }
            }

            PermissionMode::AcceptEdits => {
                if ACCEPT_EDITS_WHITELIST.contains(&tool_name) {
                    PermissionDecision::Allow
                } else if DEFAULT_SAFE_TOOLS.contains(&tool_name) {
                    PermissionDecision::Allow
                } else {
                    PermissionDecision::NeedsPermission(format!(
                        "Tool '{tool_name}' wants to: {description}"
                    ))
                }
            }

            PermissionMode::Auto => {
                // Heuristic: safe read-only tools are auto-approved,
                // everything else needs permission (will be expanded with
                // command classification in future).
                if DEFAULT_SAFE_TOOLS.contains(&tool_name) {
                    PermissionDecision::Allow
                } else {
                    PermissionDecision::NeedsPermission(format!(
                        "Tool '{tool_name}' wants to: {description}"
                    ))
                }
            }

            PermissionMode::Default => {
                if DEFAULT_SAFE_TOOLS.contains(&tool_name) {
                    PermissionDecision::Allow
                } else {
                    PermissionDecision::NeedsPermission(format!(
                        "Tool '{tool_name}' wants to: {description}"
                    ))
                }
            }
        }
    }
}
