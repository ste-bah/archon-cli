//! TASK-P1-2 (#187) — verify the native provider registry has at least 9
//! entries. Guard against silent drops of a native provider. If a NEW
//! native provider is added (count > 9) the test still passes since the
//! assertion is >= 9 (not ==).

use archon_llm::providers::native_registry::{count_native, list_native};

#[test]
fn native_provider_count_is_at_least_nine() {
    let count = count_native();
    assert!(
        count >= 9,
        "native provider count is {count}, expected >= 9. \
         Something dropped a provider from NATIVE_REGISTRY."
    );
}

#[test]
fn native_provider_list_contains_canonical_nine() {
    // Canonical list per TASK-AGS-704 module docstring. If any of these
    // disappears from NATIVE_REGISTRY, we want a specific failure, not
    // just a count mismatch.
    let names: Vec<&str> = list_native().iter().map(|d| d.id.as_str()).collect();
    for expected in &[
        "openai",
        "anthropic",
        "gemini",
        "xai",
        "bedrock",
        "azure",
        "cohere",
        "copilot",
        "minimax",
    ] {
        assert!(
            names.contains(expected),
            "native provider '{expected}' missing from NATIVE_REGISTRY. \
             Current set: {names:?}"
        );
    }
}

#[test]
fn native_provider_descriptors_are_unique_by_id() {
    let names: Vec<&str> = list_native().iter().map(|d| d.id.as_str()).collect();
    let mut sorted = names.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(
        names.len(),
        sorted.len(),
        "NATIVE_REGISTRY has duplicate provider ids: {names:?}"
    );
}
