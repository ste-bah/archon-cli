//! Coverage for v0.1.21 DEFAULT_SAFE_TOOLS expansion (7 → 25 tools).
//!
//! Selection criteria: read-only, local-only, no shell/network/filesystem
//! mutation, no destructive external side effects.

use archon_permissions::checker::PermissionChecker;
use archon_permissions::mode::{PermissionDecision, PermissionMode};
use archon_permissions::rules::{RuleSet, ToolRule};

const ALL_NEWLY_SAFE: &[&str] = &[
    "memory_store",
    "memory_recall",
    "Sleep",
    "TodoWrite",
    "ExitPlanMode",
    "lsp",
    "CartographerScan",
    "CronList",
    "ListMcpResources",
    "LeannSearch",
    "LeannFindSimilar",
    "TaskGet",
    "TaskList",
    "TaskOutput",
    "TaskCreate",
    "TaskUpdate",
    "TaskStop",
    "PushNotification",
];

#[test]
fn every_newly_safe_tool_allowed_in_default_mode() {
    let checker = PermissionChecker::new(PermissionMode::Default, RuleSet::empty());
    for tool in ALL_NEWLY_SAFE {
        assert_eq!(
            checker.check(tool, "test", "{}"),
            PermissionDecision::Allow,
            "tool {tool} must auto-allow in default mode after v0.1.21"
        );
    }
}

#[test]
fn explicit_deny_still_overrides_safe_list() {
    let mut rules = RuleSet::empty();
    rules.always_deny.push(ToolRule {
        tool: "memory_store".to_string(),
        pattern: "*".to_string(),
    });
    let checker = PermissionChecker::new(PermissionMode::Default, rules);
    assert!(matches!(
        checker.check("memory_store", "x", "{}"),
        PermissionDecision::Deny(_)
    ));
}

#[test]
fn dangerous_tools_still_gated_in_default() {
    let checker = PermissionChecker::new(PermissionMode::Default, RuleSet::empty());
    for tool in [
        "Bash",
        "PowerShell",
        "Write",
        "Edit",
        "RemoteTrigger",
        "CronCreate",
    ] {
        let decision = checker.check(tool, "test", r#"{"command":"ls"}"#);
        assert!(
            !matches!(decision, PermissionDecision::Allow),
            "tool {tool} MUST NOT be safe-listed; got {decision:?}"
        );
    }
}
