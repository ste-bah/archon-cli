//! Tests for model_picker fuzzy filter.

use archon_tui::screens::model_picker::{ModelPicker, ProviderEntry};

fn entry(provider: &str, model: &str) -> ProviderEntry {
    ProviderEntry {
        provider_id: provider.to_string(),
        model_id: model.to_string(),
        label: format!("{}/{}", provider, model),
    }
}

#[test]
fn new_picker_empty() {
    let picker = ModelPicker::new();
    assert!(picker.is_empty());
    assert_eq!(picker.query(), "");
}

#[test]
fn set_providers_updates_list() {
    let mut picker = ModelPicker::new();
    picker.set_providers(vec![
        entry("anthropic", "claude-opus"),
        entry("openai", "gpt-5"),
    ]);
    assert_eq!(picker.len(), 2);
}

#[test]
fn set_query_filters_list() {
    let mut picker = ModelPicker::new();
    picker.set_providers(vec![
        entry("anthropic", "claude-opus"),
        entry("anthropic", "claude-sonnet"),
        entry("openai", "gpt-5"),
    ]);
    picker.set_query("sonnet");
    assert_eq!(picker.len(), 1); // only claude-sonnet matches
    picker.set_query("claude");
    assert_eq!(picker.len(), 2); // both anthropic entries match
}

#[test]
fn cursor_wraps() {
    let mut picker = ModelPicker::new();
    picker.set_providers(vec![entry("a", "b"), entry("c", "d")]);
    picker.move_down();
    assert_eq!(picker.selected_index(), 1);
    picker.move_down();
    assert_eq!(picker.selected_index(), 0); // wrap
}

#[test]
fn empty_query_shows_all() {
    let mut picker = ModelPicker::new();
    picker.set_providers(vec![entry("x", "y")]);
    picker.set_query("");
    assert_eq!(picker.len(), 1);
}

#[test]
fn case_insensitive_filter() {
    let mut picker = ModelPicker::new();
    picker.set_providers(vec![
        entry("Anthropic", "Claude-Opus"),
        entry("openai", "GPT-5"),
    ]);
    picker.set_query("CLAUDE");
    assert_eq!(picker.len(), 1);
    picker.set_query("anthropic");
    assert_eq!(picker.len(), 1);
}

#[test]
fn selected_returns_entry() {
    let mut picker = ModelPicker::new();
    picker.set_providers(vec![
        entry("anthropic", "claude-opus"),
        entry("openai", "gpt-5"),
    ]);
    assert!(picker.selected().is_some());
    assert_eq!(picker.selected().unwrap().model_id, "claude-opus");
}

#[test]
fn filter_with_empty_providers() {
    let mut picker = ModelPicker::new();
    picker.set_query("anything");
    assert_eq!(picker.len(), 0);
}

#[test]
fn move_up_from_first_wraps_to_last() {
    let mut picker = ModelPicker::new();
    picker.set_providers(vec![entry("a", "b"), entry("c", "d")]);
    picker.move_up();
    assert_eq!(picker.selected_index(), 1); // wrapped to last
}
