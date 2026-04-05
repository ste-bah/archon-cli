//! Unit test for TASK-WIRE-009: config.voice.toggle_mode selects hotkey
//! behavior (toggle vs push-to-talk one-shot).
//!
//! This test exercises the pure selection function `hotkey_action_for_mode`
//! that must be called by both the TUI hotkey handler and the main binary
//! wiring log, proving `voice.toggle_mode` is a load-bearing config flag.

use archon_tui::voice::pipeline::{HotkeyAction, hotkey_action_for_mode};

#[test]
fn toggle_mode_true_returns_toggle_action() {
    assert_eq!(hotkey_action_for_mode(true), HotkeyAction::Toggle);
}

#[test]
fn toggle_mode_false_returns_push_to_talk_action() {
    assert_eq!(hotkey_action_for_mode(false), HotkeyAction::PushToTalk);
}

#[test]
fn toggle_and_push_to_talk_are_distinct() {
    assert_ne!(
        hotkey_action_for_mode(true),
        hotkey_action_for_mode(false)
    );
}
