//! Selection of the shipped self-check hooks by hook id, not by condition.

use super::repo_root;

/// Summaries of the shipped self-check hooks, selected by hook id. The id is
/// derived from the raw command, so another hook with the same condition or
/// the same redacted label (`bash`) is never counted as a self-check.
pub(super) fn self_check_survivors(
    registry: &crate::hooks::HookRegistry,
) -> Vec<crate::hooks::HookSummary> {
    use crate::hooks::{HookEvent, compute_hook_id};
    let path = repo_root().join(".archon").join("hooks.toml");
    let settings = crate::hooks::load_hooks_from_toml(&path).expect(".archon/hooks.toml parses");
    let ids: std::collections::HashSet<String> = settings[&HookEvent::PostToolUse]
        .iter()
        .flat_map(|m| m.hooks.iter().map(move |h| (m, h)))
        .filter(|(_, h)| h.command.contains("self-check-file.sh"))
        .map(|(m, h)| {
            let matcher = m.matcher.as_deref();
            compute_hook_id(&HookEvent::PostToolUse, &h.hook_type, &h.command, matcher)
        })
        .collect();
    assert_eq!(ids.len(), 3, "three distinct shipped self-check ids");
    let summaries = registry.summaries().into_iter();
    summaries.filter(|s| ids.contains(&s.id)).collect()
}

fn registry_with(hooks: &[(crate::hooks::HookEvent, &str, &str)]) -> crate::hooks::HookRegistry {
    let registry = crate::hooks::HookRegistry::new();
    for (event, command, condition) in hooks {
        let hook: crate::hooks::HookConfig = serde_json::from_value(serde_json::json!({
            "type": "prompt", "command": command, "if_condition": condition, "timeout": 5
        }))
        .expect("hook config");
        let matchers = vec![crate::hooks::HookMatcher {
            matcher: None,
            hooks: vec![hook],
        }];
        registry.register_matchers(event.clone(), matchers, Some("project"));
    }
    registry
}

const CHECK: &str = "bash scripts/self-check-file.sh";

#[test]
fn self_check_selection_ignores_decoys_with_the_same_condition() {
    use crate::hooks::HookEvent::PostToolUse as Post;
    // The self-checks collapsed to one; two other `bash` hooks use the
    // same conditions. Selection by condition would count three.
    let registry = registry_with(&[
        (Post, "bash lint.sh", "Write"),
        (Post, "bash fmt.sh", "Edit"),
        (Post, &format!("{CHECK} NotebookEdit"), "NotebookEdit"),
    ]);
    assert_eq!(self_check_survivors(&registry).len(), 1);
}

#[test]
fn self_check_selection_ignores_the_same_command_on_another_event() {
    use crate::hooks::HookEvent::{PostToolUse as Post, PreToolUse as Pre};
    let registry = registry_with(&[
        (Pre, &format!("{CHECK} Write"), "Write"),
        (Pre, &format!("{CHECK} Edit"), "Edit"),
        (Post, &format!("{CHECK} NotebookEdit"), "NotebookEdit"),
    ]);
    assert_eq!(self_check_survivors(&registry).len(), 1);
}

#[test]
fn self_check_selection_finds_each_shipped_hook_once_beside_decoys() {
    use crate::hooks::HookEvent::PostToolUse as Post;
    let registry = registry_with(&[
        (Post, &format!("{CHECK} Edit"), "Edit"),
        (Post, "bash decoy.sh", "Write"),
        (Post, &format!("{CHECK} Write"), "Write"),
        (Post, &format!("{CHECK} NotebookEdit"), "NotebookEdit"),
    ]);
    let mut conditions: Vec<_> = self_check_survivors(&registry)
        .into_iter()
        .map(|s| s.if_condition.expect("condition"))
        .collect();
    conditions.sort_unstable();
    assert_eq!(conditions, ["Edit", "NotebookEdit", "Write"]);
}
