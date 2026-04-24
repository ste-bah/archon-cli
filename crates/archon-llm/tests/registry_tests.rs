//! TASK-AGS-702 Gate 1: integration tests for the 31-entry
//! `OPENAI_COMPAT_REGISTRY`. Written before implementation.
//!
//! These tests pin the public registry surface so downstream tasks
//! (706 dispatcher, 711 final audit) can rely on it without fighting
//! module privacy. Every id, every auth classification, every base URL
//! parses, and the hand-written spec list drives the assertions — not
//! the implementation's own copy.

use archon_llm::providers::{
    AuthFlavor, CompatKind, OPENAI_COMPAT_REGISTRY, count_compat, get_compat, list_compat,
};
use url::Url;

// Spec 01-functional-spec.md line 1814 — canonical lowercase ids.
// Kept as an array so a test iterates it exactly once.
const EXPECTED_IDS: &[&str] = &[
    "ollama",
    "lm_studio",
    "llama_cpp",
    "deepseek",
    "groq",
    "xai",
    "deepinfra",
    "cerebras",
    "together_ai",
    "perplexity",
    "openrouter",
    "mistral",
    "sambanova",
    "huggingface",
    "nvidia",
    "siliconflow",
    "moonshot",
    "zhipu",
    "zai",
    "nebius",
    "novita",
    "ovhcloud",
    "scaleway",
    "vultr",
    "baseten",
    "friendli",
    "upstage",
    "stepfun",
    "fireworks",
    "qwen",
    "venice",
];

const LOCAL_IDS: &[&str] = &["ollama", "lm_studio", "llama_cpp"];

#[test]
fn registry_has_31_entries() {
    assert_eq!(OPENAI_COMPAT_REGISTRY.len(), 31);
    assert_eq!(count_compat(), 31);
    assert_eq!(EXPECTED_IDS.len(), 31, "test data invariant");
}

#[test]
fn all_spec_ids_present() {
    for id in EXPECTED_IDS {
        assert!(get_compat(id).is_some(), "registry missing spec id: {id}");
    }
}

#[test]
fn list_compat_returns_all_entries() {
    let all = list_compat();
    assert_eq!(all.len(), 31);
    // Every id from list_compat must match one of the spec ids.
    for d in all {
        assert!(
            EXPECTED_IDS.contains(&d.id.as_str()),
            "registry emitted unexpected id: {}",
            d.id
        );
    }
}

#[test]
fn local_providers_have_no_auth() {
    for id in LOCAL_IDS {
        let d = get_compat(id).unwrap_or_else(|| panic!("missing local provider: {id}"));
        assert_eq!(
            d.auth_flavor,
            AuthFlavor::None,
            "{id} must use AuthFlavor::None"
        );
        assert!(
            d.env_key_var.is_empty(),
            "{id} must have empty env_key_var, got: {}",
            d.env_key_var
        );
    }
}

#[test]
fn remote_providers_have_bearer_auth() {
    for id in EXPECTED_IDS {
        if LOCAL_IDS.contains(id) {
            continue;
        }
        let d = get_compat(id).unwrap();
        assert_eq!(
            d.auth_flavor,
            AuthFlavor::BearerApiKey,
            "{id} must use AuthFlavor::BearerApiKey"
        );
        assert!(
            !d.env_key_var.is_empty(),
            "{id} must declare a non-empty env_key_var"
        );
    }
}

#[test]
fn every_entry_has_parseable_base_url() {
    for d in list_compat() {
        // `base_url` is already `Url`, but reparse-round-trip its string
        // form to guarantee it survives the descriptor round-trip.
        let s = d.base_url.as_str();
        Url::parse(s).unwrap_or_else(|e| panic!("{} base_url {s} failed: {e}", d.id));
    }
}

#[test]
fn every_entry_is_openai_compat() {
    for d in list_compat() {
        assert_eq!(
            d.compat_kind,
            CompatKind::OpenAiCompat,
            "{} must be OpenAiCompat",
            d.id
        );
    }
}

#[test]
fn every_entry_has_default_model() {
    for d in list_compat() {
        assert!(
            !d.default_model.is_empty(),
            "{} must declare a default_model",
            d.id
        );
    }
}

#[test]
fn local_base_urls_point_to_localhost() {
    // Spec lines 31-31 of TASK-702 pin the three local URLs.
    let cases: &[(&str, &str)] = &[
        ("ollama", "http://localhost:11434/v1"),
        ("lm_studio", "http://localhost:1234/v1"),
        ("llama_cpp", "http://localhost:8080/v1"),
    ];
    for (id, url) in cases {
        let d = get_compat(id).unwrap();
        assert_eq!(d.base_url.as_str(), *url, "{id} base_url drifted from spec");
    }
}

#[test]
fn no_duplicate_env_key_vars_across_remote_providers() {
    use std::collections::HashSet;
    let mut seen: HashSet<&str> = HashSet::new();
    for d in list_compat() {
        if d.env_key_var.is_empty() {
            continue;
        }
        assert!(
            seen.insert(d.env_key_var.as_str()),
            "duplicate env_key_var across descriptors: {}",
            d.env_key_var
        );
    }
}
