//! Configuration of the repeat-tool chain (#200 Phase 2).
//!
//! The validation half: what `[guard.repeat_tool]` accepts, what it refuses,
//! and the promise that a refusal is loud rather than a silent fall back to the
//! defaults.

use super::*;

#[test]
fn the_defaults_are_the_documented_ones() {
    let config = RepeatToolConfig::default();
    assert!(config.enabled);
    assert_eq!(config.thresholds, vec![3, 5, 8]);
    assert_eq!(config.exclude, vec!["TodoWrite".to_string()]);
    assert_eq!(config.arguments_preview_chars, 500);
    assert!(config.validate().is_ok());
}

#[test]
fn an_empty_threshold_list_is_an_error_and_not_a_fallback() {
    let config = RepeatToolConfig {
        thresholds: Vec::new(),
        ..RepeatToolConfig::default()
    };
    let message = config.validate().expect_err("empty list must be rejected");
    assert!(
        message.contains("at least one run length"),
        "got: {message}"
    );
}

#[test]
fn a_threshold_below_two_is_an_error() {
    let config = RepeatToolConfig {
        thresholds: vec![1, 5],
        ..RepeatToolConfig::default()
    };
    let message = config.validate().expect_err("1 must be rejected");
    assert!(message.contains("at least 2"), "got: {message}");
}

#[test]
fn a_duplicated_threshold_is_an_error() {
    let config = RepeatToolConfig {
        thresholds: vec![3, 3, 5],
        ..RepeatToolConfig::default()
    };
    let message = config.validate().expect_err("a duplicate must be rejected");
    assert!(message.contains("repeat a value"), "got: {message}");
}

#[test]
fn a_descending_threshold_list_is_an_error() {
    let config = RepeatToolConfig {
        thresholds: vec![5, 3],
        ..RepeatToolConfig::default()
    };
    let message = config.validate().expect_err("descending must be rejected");
    assert!(message.contains("must ascend"), "got: {message}");
}

#[test]
fn a_zero_length_preview_is_an_error() {
    let config = RepeatToolConfig {
        arguments_preview_chars: 0,
        ..RepeatToolConfig::default()
    };
    let message = config.validate().expect_err("0 must be rejected");
    assert!(message.contains("at least 1"), "got: {message}");
}

/// A disabled guard with a broken list is still a broken list. Finding out on
/// the day it is switched on is finding out late.
#[test]
fn a_disabled_guard_still_validates_its_thresholds() {
    let config = RepeatToolConfig {
        enabled: false,
        thresholds: vec![1],
        ..RepeatToolConfig::default()
    };
    assert!(config.validate().is_err());
}

#[test]
fn a_chain_key_separates_a_subagent_from_its_parent_context() {
    let mut parent = ToolContext {
        session_id: "session-a".to_string(),
        ..ToolContext::default()
    };
    let child = {
        let mut ctx = parent.clone();
        ctx.subagent_id = Some("child-1".to_string());
        ctx
    };
    assert_ne!(ChainKey::of(&parent), ChainKey::of(&child));
    parent.subagent_id = None;
    assert_eq!(ChainKey::of(&parent), ChainKey::new("session-a", None));
}
