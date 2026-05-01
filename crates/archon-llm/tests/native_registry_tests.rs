//! GHOST-003: integration tests for `NATIVE_REGISTRY` — 5 native
//! providers (openai, anthropic, gemini, xai, bedrock). The 4 stub
//! providers (azure, cohere, copilot, minimax) were removed per
//! GHOST-003 Option B.
//!
//! Validation criteria:
//!   (2) NATIVE_REGISTRY has exactly 5 entries
//!   (3) all 5 ids retrievable
//!   (4) combined breadth >= 36
//!   (6) every entry has CompatKind::Native

use archon_llm::providers::{CompatKind, NATIVE_REGISTRY, count_compat, count_native, list_native};

// ---------------------------------------------------------------------------
// Expected native ids (GHOST-003: 4 stubs removed)
// ---------------------------------------------------------------------------

const EXPECTED_NATIVE_IDS: &[&str] = &["openai", "anthropic", "gemini", "xai", "bedrock"];

// ---------------------------------------------------------------------------
// Registry shape / breadth tests
// ---------------------------------------------------------------------------

#[test]
fn native_registry_has_5_entries() {
    assert_eq!(
        NATIVE_REGISTRY.len(),
        5,
        "GHOST-003 requires exactly 5 native descriptors (openai, anthropic, gemini, xai, bedrock)"
    );
    assert_eq!(count_native(), 5);
}

#[test]
fn all_native_ids_present() {
    for id in EXPECTED_NATIVE_IDS {
        assert!(
            NATIVE_REGISTRY.contains_key(*id),
            "native registry missing id `{id}` — GHOST-003 requires all 5"
        );
    }
}

#[test]
fn list_native_returns_all_entries() {
    let all = list_native();
    assert_eq!(all.len(), 5);
    for id in EXPECTED_NATIVE_IDS {
        assert!(
            all.iter().any(|d| d.id == *id),
            "list_native() missing id `{id}`"
        );
    }
}

#[test]
fn every_entry_is_native() {
    for (id, desc) in NATIVE_REGISTRY.iter() {
        assert_eq!(
            desc.compat_kind,
            CompatKind::Native,
            "descriptor `{id}` must have CompatKind::Native"
        );
    }
}

#[test]
fn every_native_entry_has_parseable_base_url() {
    for (id, desc) in NATIVE_REGISTRY.iter() {
        let s = desc.base_url.as_str();
        assert!(
            s.starts_with("http://") || s.starts_with("https://"),
            "descriptor `{id}` base_url `{s}` must be http(s)"
        );
    }
}

#[test]
fn every_native_entry_has_default_model() {
    for (id, desc) in NATIVE_REGISTRY.iter() {
        assert!(
            !desc.default_model.is_empty(),
            "descriptor `{id}` default_model must not be empty"
        );
    }
}

#[test]
fn combined_breadth_ge_36() {
    let native = count_native();
    let compat = count_compat();
    let total = native + compat;
    assert!(
        total >= 36,
        "GHOST-003 invariant: native ({native}) + compat ({compat}) = {total}, must be >= 36"
    );
}

#[test]
fn native_ids_are_unique() {
    let mut ids: Vec<&str> = NATIVE_REGISTRY.keys().copied().collect();
    ids.sort();
    let len = ids.len();
    ids.dedup();
    assert_eq!(len, ids.len(), "duplicate native ids detected");
}
