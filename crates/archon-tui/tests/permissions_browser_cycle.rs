use archon_tui::screens::permissions_browser::{PermissionsBrowser, ToolPermission};

#[test]
fn test_cycle_allow_to_deny() {
    let mut b = PermissionsBrowser::new();
    b.set_permissions(vec![ToolPermission::Allow("a".into())]);
    b.cycle_selected();
    assert!(matches!(b.selected(), Some(ToolPermission::Deny(_))));
}

#[test]
fn test_cycle_deny_to_prompt() {
    let mut b = PermissionsBrowser::new();
    b.set_permissions(vec![ToolPermission::Deny("a".into())]);
    b.cycle_selected();
    assert!(matches!(b.selected(), Some(ToolPermission::Prompt(_))));
}

#[test]
fn test_cycle_prompt_wraps_to_allow() {
    let mut b = PermissionsBrowser::new();
    b.set_permissions(vec![ToolPermission::Prompt("a".into())]);
    b.cycle_selected();
    assert!(matches!(b.selected(), Some(ToolPermission::Allow(_))));
}

#[test]
fn test_empty_browser_no_panic() {
    let mut b = PermissionsBrowser::new();
    b.cycle_selected();
    b.move_up();
    b.move_down();
    assert!(b.selected().is_none());
}
