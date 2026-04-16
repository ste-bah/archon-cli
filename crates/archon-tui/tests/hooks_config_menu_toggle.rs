//! Integration tests for hooks_config_menu Space-toggle behavior.

use archon_tui::screens::hooks_config_menu::{HookSpec, HooksMenu, HookStore};

/// In-memory store for testing.
struct TestHookStore {
    hooks: Vec<HookSpec>,
}

impl TestHookStore {
    fn new(hooks: Vec<HookSpec>) -> Self {
        Self { hooks }
    }
}

impl HookStore for TestHookStore {
    fn save_hook_enabled(&self, _name: &str, _enabled: bool) -> bool {
        // Persistence is a no-op in this test store
        true
    }

    fn load_hooks(&self) -> Vec<HookSpec> {
        self.hooks.clone()
    }
}

fn make_hook(name: &str, enabled: bool) -> HookSpec {
    HookSpec {
        name: name.into(),
        enabled,
        script_path: format!("/hooks/{}", name),
    }
}

#[test]
fn test_toggle_selected_hook_enabled_state() {
    let store = TestHookStore::new(vec![make_hook("pre-commit", true)]);
    let mut menu = HooksMenu::new();
    menu.set_hooks(store.load_hooks());

    assert!(menu.selected().unwrap().enabled);
    menu.toggle_selected();
    assert!(!menu.selected().unwrap().enabled);
    menu.toggle_selected();
    assert!(menu.selected().unwrap().enabled);
}

#[test]
fn test_toggle_after_loading_from_store() {
    // Simulate loading hooks from store and toggling
    let store = TestHookStore::new(vec![
        make_hook("enabled-hook", true),
        make_hook("disabled-hook", false),
    ]);
    let mut menu = HooksMenu::new();
    menu.set_hooks(store.load_hooks());

    assert_eq!(menu.len(), 2);
    assert_eq!(menu.selected_index(), 0);
    assert!(menu.selected().unwrap().enabled); // first is enabled

    menu.toggle_selected(); // disable first
    assert!(!menu.selected().unwrap().enabled);

    menu.move_down();
    assert_eq!(menu.selected_index(), 1);
    assert!(!menu.selected().unwrap().enabled); // second is disabled

    menu.toggle_selected(); // enable second
    assert!(menu.selected().unwrap().enabled);
}

#[test]
fn test_menu_cursor_wraps() {
    let store = TestHookStore::new(vec![
        make_hook("hook-a", true),
        make_hook("hook-b", false),
    ]);
    let mut menu = HooksMenu::new();
    menu.set_hooks(store.load_hooks());

    assert_eq!(menu.selected_index(), 0);
    menu.move_down();
    assert_eq!(menu.selected_index(), 1);
    menu.move_down();
    assert_eq!(menu.selected_index(), 0); // wraps
}

#[test]
fn test_hook_list_len() {
    let store = TestHookStore::new(vec![
        make_hook("enabled-hook", true),
        make_hook("disabled-hook", false),
    ]);
    let mut menu = HooksMenu::new();
    menu.set_hooks(store.load_hooks());

    assert_eq!(menu.len(), 2);
}
