use crate::checker::PermissionChecker;
use crate::mode::{PermissionDecision, PermissionMode};
use crate::rules::{RuleSet, ToolRule};

fn deny_bash_rules() -> RuleSet {
    let mut rules = RuleSet::empty();
    rules.always_deny.push(ToolRule {
        tool: "Bash".to_string(),
        pattern: "*".to_string(),
    });
    rules
}

#[test]
fn always_deny_rules_are_absolute_across_elevated_modes() {
    for mode in [
        PermissionMode::Default,
        PermissionMode::AcceptEdits,
        PermissionMode::Auto,
        PermissionMode::DontAsk,
        PermissionMode::Bubble,
        PermissionMode::BypassPermissions,
    ] {
        let checker = PermissionChecker::new(mode, deny_bash_rules());
        let decision = checker.check("Bash", "run shell command", "cargo test");

        assert!(
            matches!(decision, PermissionDecision::Deny(_)),
            "always_deny must block Bash in {mode}; got {decision:?}"
        );
    }
}

#[test]
fn allow_rules_do_not_override_plan_mode_deny_rules() {
    let mut rules = RuleSet::empty();
    rules.always_allow.push(ToolRule {
        tool: "Bash".to_string(),
        pattern: "*".to_string(),
    });
    rules.always_deny.push(ToolRule {
        tool: "Bash".to_string(),
        pattern: "rm:*".to_string(),
    });

    let checker = PermissionChecker::new(PermissionMode::Plan, rules);

    assert!(matches!(
        checker.check("Bash", "remove files", "rm -rf target"),
        PermissionDecision::Deny(_)
    ));
}

#[test]
fn codex_or_anthropic_identity_is_not_part_of_permission_decisions() {
    let checker = PermissionChecker::new(PermissionMode::Default, RuleSet::empty());

    assert_eq!(
        checker.check("Read", "read file", r#"{"file_path":"src/lib.rs"}"#),
        PermissionDecision::Allow
    );
    assert!(matches!(
        checker.check("Bash", "run shell", r#"{"command":"cargo check"}"#),
        PermissionDecision::NeedsPermission(_)
    ));
}
