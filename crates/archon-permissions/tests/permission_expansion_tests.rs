use archon_permissions::checker::PermissionChecker;
use archon_permissions::denial_log::DenialLog;
use archon_permissions::mode::{PermissionDecision, PermissionMode};
use archon_permissions::rules::{RuleSet, ToolRule};

// -----------------------------------------------------------------------
// Mode tests
// -----------------------------------------------------------------------

#[test]
fn default_mode_prompts_for_bash() {
    let checker = PermissionChecker::new(PermissionMode::Default, RuleSet::empty());
    let result = checker.check("Bash", "run a shell command", "ls -la");
    assert!(
        matches!(result, PermissionDecision::NeedsPermission(_)),
        "Default mode should prompt for Bash, got: {result:?}"
    );
}

#[test]
fn default_mode_allows_read() {
    let checker = PermissionChecker::new(PermissionMode::Default, RuleSet::empty());
    let result = checker.check("Read", "read a file", "/tmp/foo.txt");
    // Default mode allows safe read-only tools without prompting
    assert_eq!(result, PermissionDecision::Allow);
}

#[test]
fn accept_edits_auto_allows_write() {
    let checker = PermissionChecker::new(PermissionMode::AcceptEdits, RuleSet::empty());
    let result = checker.check("Write", "write a file", "/tmp/foo.txt");
    assert_eq!(result, PermissionDecision::Allow);
}

#[test]
fn accept_edits_prompts_for_bash() {
    let checker = PermissionChecker::new(PermissionMode::AcceptEdits, RuleSet::empty());
    let result = checker.check("Bash", "run a command", "cargo build");
    assert!(
        matches!(result, PermissionDecision::NeedsPermission(_)),
        "AcceptEdits should prompt for Bash, got: {result:?}"
    );
}

#[test]
fn plan_mode_allows_read() {
    let checker = PermissionChecker::new(PermissionMode::Plan, RuleSet::empty());
    let result = checker.check("Read", "read a file", "/tmp/foo.txt");
    assert_eq!(result, PermissionDecision::Allow);
}

#[test]
fn plan_mode_allows_glob() {
    let checker = PermissionChecker::new(PermissionMode::Plan, RuleSet::empty());
    let result = checker.check("Glob", "search files", "**/*.rs");
    assert_eq!(result, PermissionDecision::Allow);
}

#[test]
fn plan_mode_allows_grep() {
    let checker = PermissionChecker::new(PermissionMode::Plan, RuleSet::empty());
    let result = checker.check("Grep", "search content", "pattern");
    assert_eq!(result, PermissionDecision::Allow);
}

#[test]
fn plan_mode_denies_write() {
    let checker = PermissionChecker::new(PermissionMode::Plan, RuleSet::empty());
    let result = checker.check("Write", "write a file", "/tmp/foo.txt");
    assert!(
        matches!(result, PermissionDecision::Deny(_)),
        "Plan mode must deny Write, got: {result:?}"
    );
}

#[test]
fn plan_mode_denies_edit() {
    let checker = PermissionChecker::new(PermissionMode::Plan, RuleSet::empty());
    let result = checker.check("Edit", "edit a file", "/tmp/foo.txt");
    assert!(
        matches!(result, PermissionDecision::Deny(_)),
        "Plan mode must deny Edit, got: {result:?}"
    );
}

#[test]
fn plan_mode_denies_bash() {
    let checker = PermissionChecker::new(PermissionMode::Plan, RuleSet::empty());
    let result = checker.check("Bash", "run a command", "ls -la");
    assert!(
        matches!(result, PermissionDecision::Deny(_)),
        "Plan mode must deny Bash, got: {result:?}"
    );
}

#[test]
fn plan_mode_denies_exit_plan_mode() {
    let checker = PermissionChecker::new(PermissionMode::Plan, RuleSet::empty());
    let result = checker.check("ExitPlanMode", "exit plan mode", "");
    assert!(
        matches!(result, PermissionDecision::Deny(_)),
        "Plan mode must deny ExitPlanMode, got: {result:?}"
    );
}

#[test]
fn plan_mode_denies_mcp_tool() {
    let checker = PermissionChecker::new(PermissionMode::Plan, RuleSet::empty());
    let result = checker.check("mcp__memorygraph__store_memory", "store memory", "{}");
    assert!(
        matches!(result, PermissionDecision::Deny(_)),
        "Plan mode must deny MCP tools, got: {result:?}"
    );
}

#[test]
fn dont_ask_allows_all() {
    let checker = PermissionChecker::new(PermissionMode::DontAsk, RuleSet::empty());
    assert_eq!(checker.check("Bash", "run rm -rf", "rm -rf /"), PermissionDecision::Allow);
    assert_eq!(checker.check("Write", "write file", "/etc/passwd"), PermissionDecision::Allow);
    assert_eq!(checker.check("Read", "read file", "/tmp/x"), PermissionDecision::Allow);
}

#[test]
fn bypass_allows_all() {
    let checker = PermissionChecker::new(PermissionMode::BypassPermissions, RuleSet::empty());
    assert_eq!(checker.check("Bash", "run rm -rf", "rm -rf /"), PermissionDecision::Allow);
    assert_eq!(checker.check("Write", "write file", "/etc/passwd"), PermissionDecision::Allow);
    assert_eq!(checker.check("Read", "read file", "/tmp/x"), PermissionDecision::Allow);
}

// -----------------------------------------------------------------------
// Legacy alias tests
// -----------------------------------------------------------------------

#[test]
fn ask_maps_to_default() {
    let mode: PermissionMode = "ask".parse().expect("ask should parse");
    assert_eq!(mode, PermissionMode::Default);
}

#[test]
fn yolo_maps_to_bypass() {
    let mode: PermissionMode = "yolo".parse().expect("yolo should parse");
    assert_eq!(mode, PermissionMode::BypassPermissions);
}

// -----------------------------------------------------------------------
// Rule tests
// -----------------------------------------------------------------------

#[test]
fn deny_rule_blocks_in_dont_ask() {
    let rules = RuleSet {
        always_allow: vec![],
        always_deny: vec![ToolRule {
            tool: "Bash".into(),
            pattern: "rm:*".into(),
        }],
        always_ask: vec![],
    };
    let checker = PermissionChecker::new(PermissionMode::DontAsk, rules);
    let result = checker.check("Bash", "remove files", "rm -rf /");
    assert!(
        matches!(result, PermissionDecision::Deny(_)),
        "Deny rule must block even in DontAsk, got: {result:?}"
    );
}

#[test]
fn allow_rule_auto_approves_in_default() {
    let rules = RuleSet {
        always_allow: vec![ToolRule {
            tool: "Bash".into(),
            pattern: "git:*".into(),
        }],
        always_deny: vec![],
        always_ask: vec![],
    };
    let checker = PermissionChecker::new(PermissionMode::Default, rules);
    let result = checker.check("Bash", "check status", "git status");
    assert_eq!(result, PermissionDecision::Allow);
}

#[test]
fn deny_takes_precedence_over_allow() {
    let rules = RuleSet {
        always_allow: vec![ToolRule {
            tool: "Bash".into(),
            pattern: "git:*".into(),
        }],
        always_deny: vec![ToolRule {
            tool: "Bash".into(),
            pattern: "git:*".into(),
        }],
        always_ask: vec![],
    };
    let checker = PermissionChecker::new(PermissionMode::DontAsk, rules);
    let result = checker.check("Bash", "git push", "git push");
    assert!(
        matches!(result, PermissionDecision::Deny(_)),
        "Deny must take precedence over allow, got: {result:?}"
    );
}

#[test]
fn wildcard_pattern_matches() {
    let rules = RuleSet {
        always_allow: vec![ToolRule {
            tool: "Bash".into(),
            pattern: "git:*".into(),
        }],
        always_deny: vec![],
        always_ask: vec![],
    };
    let checker = PermissionChecker::new(PermissionMode::Default, rules);

    // "git status" starts with "git" so "git:*" matches
    assert_eq!(
        checker.check("Bash", "git status", "git status"),
        PermissionDecision::Allow
    );
    // "git commit" starts with "git" so "git:*" matches
    assert_eq!(
        checker.check("Bash", "git commit", "git commit -m 'msg'"),
        PermissionDecision::Allow
    );
    // "rm -rf" does not start with "git"
    let result = checker.check("Bash", "remove all", "rm -rf /");
    assert!(
        !matches!(result, PermissionDecision::Allow)
            || matches!(result, PermissionDecision::NeedsPermission(_)),
        "git:* should not match rm -rf"
    );
}

#[test]
fn no_rule_falls_through_to_mode() {
    let rules = RuleSet {
        always_allow: vec![ToolRule {
            tool: "Bash".into(),
            pattern: "git:*".into(),
        }],
        always_deny: vec![],
        always_ask: vec![],
    };
    // No rule for Write tool, so mode logic applies
    let checker = PermissionChecker::new(PermissionMode::Default, rules);
    let result = checker.check("Write", "write file", "/tmp/foo");
    // Default mode prompts for Write
    assert!(
        matches!(result, PermissionDecision::NeedsPermission(_)),
        "No matching rule should fall through to mode logic, got: {result:?}"
    );
}

// -----------------------------------------------------------------------
// Mode cycling
// -----------------------------------------------------------------------

#[test]
fn mode_cycle_without_bypass() {
    let mut mode = PermissionMode::Default;
    mode = mode.next_mode(false);
    assert_eq!(mode, PermissionMode::AcceptEdits);
    mode = mode.next_mode(false);
    assert_eq!(mode, PermissionMode::Plan);
    mode = mode.next_mode(false);
    assert_eq!(mode, PermissionMode::Auto);
    mode = mode.next_mode(false);
    assert_eq!(mode, PermissionMode::DontAsk);
    mode = mode.next_mode(false);
    assert_eq!(mode, PermissionMode::Default); // wraps around, skipping bypass
}

#[test]
fn mode_cycle_with_bypass() {
    let mut mode = PermissionMode::DontAsk;
    mode = mode.next_mode(true);
    assert_eq!(mode, PermissionMode::BypassPermissions);
    mode = mode.next_mode(true);
    assert_eq!(mode, PermissionMode::Default); // wraps around
}

// -----------------------------------------------------------------------
// Denial log
// -----------------------------------------------------------------------

#[test]
fn denial_log_records() {
    let mut log = DenialLog::new();
    log.record("Bash", "dangerous command");
    log.record("Write", "path denied");
    log.record("Edit", "plan mode");
    assert_eq!(log.recent(3).len(), 3);
    assert_eq!(log.recent(2).len(), 2);
    assert_eq!(log.recent(10).len(), 3);
}

#[test]
fn denial_log_format() {
    let mut log = DenialLog::new();
    log.record("Bash", "dangerous command");
    log.record("Write", "path denied");
    let output = log.format_display(5);
    assert!(output.contains("Bash"));
    assert!(output.contains("dangerous command"));
    assert!(output.contains("Write"));
    assert!(output.contains("path denied"));
}

// -----------------------------------------------------------------------
// Additional plan mode edge cases
// -----------------------------------------------------------------------

#[test]
fn plan_mode_allows_ask_user_question() {
    let checker = PermissionChecker::new(PermissionMode::Plan, RuleSet::empty());
    let result = checker.check("AskUserQuestion", "ask user", "what do you think?");
    assert_eq!(result, PermissionDecision::Allow);
}

#[test]
fn plan_mode_allows_enter_plan_mode() {
    let checker = PermissionChecker::new(PermissionMode::Plan, RuleSet::empty());
    let result = checker.check("EnterPlanMode", "enter plan mode", "");
    assert_eq!(result, PermissionDecision::Allow);
}

#[test]
fn plan_mode_allows_tool_search() {
    let checker = PermissionChecker::new(PermissionMode::Plan, RuleSet::empty());
    let result = checker.check("ToolSearch", "search tools", "query");
    assert_eq!(result, PermissionDecision::Allow);
}

// -----------------------------------------------------------------------
// FromStr / canonical name parsing
// -----------------------------------------------------------------------

#[test]
fn parse_all_canonical_names() {
    assert_eq!("default".parse::<PermissionMode>().unwrap(), PermissionMode::Default);
    assert_eq!("acceptEdits".parse::<PermissionMode>().unwrap(), PermissionMode::AcceptEdits);
    assert_eq!("plan".parse::<PermissionMode>().unwrap(), PermissionMode::Plan);
    assert_eq!("auto".parse::<PermissionMode>().unwrap(), PermissionMode::Auto);
    assert_eq!("dontAsk".parse::<PermissionMode>().unwrap(), PermissionMode::DontAsk);
    assert_eq!(
        "bypassPermissions".parse::<PermissionMode>().unwrap(),
        PermissionMode::BypassPermissions
    );
}

#[test]
fn parse_invalid_mode_fails() {
    assert!("invalid".parse::<PermissionMode>().is_err());
    assert!("YOLO".parse::<PermissionMode>().is_err());
    assert!("Ask".parse::<PermissionMode>().is_err());
}

// -----------------------------------------------------------------------
// AcceptEdits allows Read, Edit, Glob, Grep
// -----------------------------------------------------------------------

#[test]
fn accept_edits_allows_read() {
    let checker = PermissionChecker::new(PermissionMode::AcceptEdits, RuleSet::empty());
    assert_eq!(checker.check("Read", "read file", "/tmp/x"), PermissionDecision::Allow);
}

#[test]
fn accept_edits_allows_edit() {
    let checker = PermissionChecker::new(PermissionMode::AcceptEdits, RuleSet::empty());
    assert_eq!(checker.check("Edit", "edit file", "/tmp/x"), PermissionDecision::Allow);
}

#[test]
fn accept_edits_allows_glob() {
    let checker = PermissionChecker::new(PermissionMode::AcceptEdits, RuleSet::empty());
    assert_eq!(checker.check("Glob", "glob search", "**/*.rs"), PermissionDecision::Allow);
}

#[test]
fn accept_edits_allows_grep() {
    let checker = PermissionChecker::new(PermissionMode::AcceptEdits, RuleSet::empty());
    assert_eq!(checker.check("Grep", "grep search", "pattern"), PermissionDecision::Allow);
}

// -----------------------------------------------------------------------
// SECURITY: Deny rules survive BypassPermissions mode
// -----------------------------------------------------------------------

#[test]
fn deny_rule_blocks_in_bypass_permissions() {
    let rules = RuleSet {
        always_allow: vec![],
        always_deny: vec![ToolRule {
            tool: "Bash".into(),
            pattern: "rm:*".into(),
        }],
        always_ask: vec![],
    };
    let checker = PermissionChecker::new(PermissionMode::BypassPermissions, rules);
    let result = checker.check("Bash", "remove files", "rm:-rf:/");
    assert!(
        matches!(result, PermissionDecision::Deny(_)),
        "Deny rule MUST block even in BypassPermissions mode, got: {result:?}"
    );
}

// -----------------------------------------------------------------------
// SECURITY: Plan mode denies PowerShell
// -----------------------------------------------------------------------

#[test]
fn plan_mode_denies_powershell() {
    let checker = PermissionChecker::new(PermissionMode::Plan, RuleSet::empty());
    let result = checker.check("PowerShell", "run powershell", "Get-Process");
    assert!(
        matches!(result, PermissionDecision::Deny(_)),
        "Plan mode MUST deny PowerShell, got: {result:?}"
    );
}
