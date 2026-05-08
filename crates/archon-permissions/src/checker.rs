use crate::mode::{PermissionDecision, PermissionMode};
use crate::rules::RuleSet;

/// Tools allowed in Plan mode (read-only + navigation + coordination).
const PLAN_MODE_WHITELIST: &[&str] = &[
    "Read",
    "Glob",
    "Grep",
    "AskUserQuestion",
    "EnterPlanMode",
    "ToolSearch",
    "Agent",
];

/// Tools auto-allowed in AcceptEdits mode (file operations + safe session tools).
const ACCEPT_EDITS_WHITELIST: &[&str] = &[
    "Read",
    "Write",
    "Edit",
    "Glob",
    "Grep",
    "ToolSearch",
    "AskUserQuestion",
    "TodoWrite",
    "Sleep",
    "Config",
    "EnterPlanMode",
    "ExitPlanMode",
    "NotebookEdit",
];

/// Tools that are safe (read-only or coordination) and auto-allowed in Default mode.
///
/// Selection criteria: read-only, local-only (no shell/network/filesystem
/// mutation), no destructive external side effects.
const DEFAULT_SAFE_TOOLS: &[&str] = &[
    // ----- Read-only file/code inspection -----
    "Read",
    "Glob",
    "Grep",
    // ----- Coordination -----
    "ToolSearch",
    "AskUserQuestion",
    "EnterPlanMode",
    "Agent",
    // ----- Memory (local CozoDB graph; no network/shell) -----
    "memory_store",
    "memory_recall",
    // ----- Sleep / planning / in-session state -----
    "Sleep",
    "TodoWrite",
    "ExitPlanMode",
    // ----- Read-only code/symbol intelligence -----
    "lsp",
    "CartographerScan",
    // ----- Read-only catalog/discovery -----
    "CronList",
    "ListMcpResources",
    // ----- Read-only evidence-engine inspection -----
    "GameTheoryStatus",
    "GameTheoryListAgents",
    "GameTheoryInspect",
    // ----- LEANN semantic code search (read-only) -----
    "LeannSearch",
    "LeannFindSimilar",
    // ----- Task lifecycle (internal coordination; no external effects) -----
    "TaskGet",
    "TaskList",
    "TaskOutput",
    "TaskCreate",
    "TaskUpdate",
    "TaskStop",
    // ----- User-visible local notifications -----
    "PushNotification",
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

    /// Evaluate fine-grained rules without falling through to mode defaults.
    ///
    /// Deny rules remain absolute. `BypassPermissions` still turns non-deny
    /// rule decisions into allow, matching [`Self::check`].
    pub fn check_rule_decision(
        &self,
        tool_name: &str,
        tool_args: &str,
    ) -> Option<PermissionDecision> {
        let decision = self.rules.evaluate(tool_name, tool_args)?;
        if matches!(decision, PermissionDecision::Deny(_)) {
            return Some(decision);
        }
        if self.mode == PermissionMode::BypassPermissions {
            return Some(PermissionDecision::Allow);
        }
        Some(decision)
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
        if let Some(decision) = self.check_rule_decision(tool_name, tool_args) {
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
                if ACCEPT_EDITS_WHITELIST.contains(&tool_name)
                    || DEFAULT_SAFE_TOOLS.contains(&tool_name)
                {
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

// ---------------------------------------------------------------------------
// Public accessors — single source of truth for mode gate lists
// ---------------------------------------------------------------------------

pub fn default_safe_tools() -> &'static [&'static str] {
    DEFAULT_SAFE_TOOLS
}

pub fn is_default_safe_tool(name: &str) -> bool {
    DEFAULT_SAFE_TOOLS.contains(&name)
}

pub fn accept_edits_whitelist() -> &'static [&'static str] {
    ACCEPT_EDITS_WHITELIST
}

pub fn is_accept_edits_safe_tool(name: &str) -> bool {
    ACCEPT_EDITS_WHITELIST.contains(&name)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_tool_allowed_in_default_mode() {
        let checker = PermissionChecker::new(PermissionMode::Default, RuleSet::empty());
        let decision = checker.check("Agent", "spawn subagent", "");
        assert_eq!(decision, PermissionDecision::Allow);
    }

    #[test]
    fn agent_tool_allowed_in_accept_edits_mode() {
        let checker = PermissionChecker::new(PermissionMode::AcceptEdits, RuleSet::empty());
        let decision = checker.check("Agent", "spawn subagent", "");
        assert_eq!(decision, PermissionDecision::Allow);
    }

    #[test]
    fn agent_tool_allowed_in_auto_mode() {
        let checker = PermissionChecker::new(PermissionMode::Auto, RuleSet::empty());
        let decision = checker.check("Agent", "spawn subagent", "");
        assert_eq!(decision, PermissionDecision::Allow);
    }

    #[test]
    fn agent_tool_allowed_in_plan_mode() {
        let checker = PermissionChecker::new(PermissionMode::Plan, RuleSet::empty());
        let decision = checker.check("Agent", "spawn subagent", "");
        assert_eq!(decision, PermissionDecision::Allow);
    }

    #[test]
    fn dangerous_tools_still_gated_in_default_mode() {
        let checker = PermissionChecker::new(PermissionMode::Default, RuleSet::empty());
        // Bash, Write, Edit should still require permission
        assert!(matches!(
            checker.check("Bash", "run command", ""),
            PermissionDecision::NeedsPermission(_)
        ));
        assert!(matches!(
            checker.check("Write", "write file", ""),
            PermissionDecision::NeedsPermission(_)
        ));
        assert!(matches!(
            checker.check("Edit", "edit file", ""),
            PermissionDecision::NeedsPermission(_)
        ));
    }

    #[test]
    fn accept_edits_preserves_session_tool_allowlist() {
        let checker = PermissionChecker::new(PermissionMode::AcceptEdits, RuleSet::empty());

        assert_eq!(
            checker.check("Config", "read config", ""),
            PermissionDecision::Allow
        );
        assert_eq!(
            checker.check("NotebookEdit", "edit notebook", ""),
            PermissionDecision::Allow
        );
    }

    #[test]
    fn deny_rule_still_blocks_agent_tool() {
        // Even though Agent is in DEFAULT_SAFE_TOOLS, an explicit deny
        // rule should still block it (deny rules are absolute).
        let mut rules = RuleSet::empty();
        rules.always_deny.push(crate::rules::ToolRule {
            tool: "Agent".to_string(),
            pattern: "*".to_string(),
        });
        let checker = PermissionChecker::new(PermissionMode::Default, rules);
        let decision = checker.check("Agent", "spawn subagent", "");
        assert!(matches!(decision, PermissionDecision::Deny(_)));
    }
}
