//! TASK-AGS-704 Gate 1: integration tests for `NATIVE_REGISTRY` and the
//! four stub native providers (`AzureProvider`, `CohereProvider`,
//! `CopilotProvider`, `MinimaxProvider`).
//!
//! Written BEFORE implementation. These pin the public behavior so the
//! impl has a compile-and-pass target.
//!
//! TASK-AGS-704 SPEC DEVIATION (inherited from the greenlit TASK-AGS-703
//! mapping, 2026-04-13):
//!   ProviderError::InvalidResponse -> LlmError::Unsupported("... Open Question #3 ...")
//!   .chat()                        -> .complete() (real trait method)
//! The spec text prescribes `ProviderError::InvalidResponse`; the real
//! `LlmProvider` trait returns `LlmError`, so the stub impls surface the
//! "Open Question #3" sentinel through `LlmError::Unsupported` instead.
//! The sentinel string is preserved so `grep 'Open Question #3'` still
//! locates every gap-filler per the TASK-AGS-704 wiring check.
//!
//! Validation criteria (TASK-AGS-704 §Validation Criteria):
//!   (2) NATIVE_REGISTRY has exactly 9 entries       -> `native_registry_has_9_entries`
//!   (3) all 9 ids retrievable                       -> `all_native_ids_present`
//!   (4) combined breadth >= 40                      -> `combined_breadth_ge_40`
//!   (5) each stub returns Unsupported("... Open Question #3 ...") -> `*_stub_returns_open_question_3`
//!   (6) every entry has CompatKind::Native          -> `every_entry_is_native`

use std::sync::Arc;

use archon_llm::ApiKey;
use archon_llm::provider::{LlmError, LlmProvider, LlmRequest};
use archon_llm::providers::{
    AzureProvider, CohereProvider, CompatKind, CopilotProvider, MinimaxProvider, NATIVE_REGISTRY,
    count_compat, count_native, list_native,
};

// ---------------------------------------------------------------------------
// Expected native ids from TASK-AGS-704 spec (line 25)
// ---------------------------------------------------------------------------

const EXPECTED_NATIVE_IDS: &[&str] = &[
    "openai",
    "anthropic",
    "gemini",
    "xai",
    "bedrock",
    "azure",
    "cohere",
    "copilot",
    "minimax",
];

// ---------------------------------------------------------------------------
// Registry shape / breadth tests (validation criteria 2-4, 6)
// ---------------------------------------------------------------------------

#[test]
fn native_registry_has_9_entries() {
    assert_eq!(
        NATIVE_REGISTRY.len(),
        9,
        "TASK-AGS-704 requires exactly 9 native descriptors (openai, anthropic, gemini, xai, bedrock, azure, cohere, copilot, minimax)"
    );
    assert_eq!(count_native(), 9);
}

#[test]
fn all_native_ids_present() {
    for id in EXPECTED_NATIVE_IDS {
        assert!(
            NATIVE_REGISTRY.contains_key(*id),
            "native registry missing id `{id}` — spec (line 25) requires all 9"
        );
    }
}

#[test]
fn list_native_returns_all_entries() {
    let all = list_native();
    assert_eq!(all.len(), 9);
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
fn combined_breadth_ge_40() {
    let native = count_native();
    let compat = count_compat();
    let total = native + compat;
    assert!(
        total >= 40,
        "TASK-AGS-704 D6 invariant: native ({native}) + compat ({compat}) = {total}, must be >= 40"
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

// ---------------------------------------------------------------------------
// Stub provider tests (validation criterion 5)
//
// Each stub must surface the "Open Question #3" sentinel in its error so
// auditors can grep for gap-fillers (TASK-AGS-704 wiring check).
// ---------------------------------------------------------------------------

fn simple_request() -> LlmRequest {
    LlmRequest {
        model: "ignored".into(),
        max_tokens: 16,
        system: Vec::new(),
        messages: vec![serde_json::json!({"role":"user","content":"hi"})],
        tools: Vec::new(),
        thinking: None,
        speed: None,
        effort: None,
        request_origin: None,
        extra: serde_json::Value::Null,
    }
}

fn assert_open_question_3(err: &LlmError, provider: &str) {
    match err {
        LlmError::Unsupported(msg) => {
            assert!(
                msg.contains("Open Question #3"),
                "{provider} stub must mention 'Open Question #3' sentinel; got: {msg}"
            );
        }
        other => panic!("{provider} stub must return LlmError::Unsupported(..); got: {other:?}"),
    }
}

fn azure_descriptor() -> &'static archon_llm::providers::ProviderDescriptor {
    NATIVE_REGISTRY
        .get("azure")
        .expect("azure descriptor must exist")
}

fn cohere_descriptor() -> &'static archon_llm::providers::ProviderDescriptor {
    NATIVE_REGISTRY
        .get("cohere")
        .expect("cohere descriptor must exist")
}

fn copilot_descriptor() -> &'static archon_llm::providers::ProviderDescriptor {
    NATIVE_REGISTRY
        .get("copilot")
        .expect("copilot descriptor must exist")
}

fn minimax_descriptor() -> &'static archon_llm::providers::ProviderDescriptor {
    NATIVE_REGISTRY
        .get("minimax")
        .expect("minimax descriptor must exist")
}

fn http() -> Arc<reqwest::Client> {
    Arc::new(reqwest::Client::new())
}

#[tokio::test]
async fn azure_stub_returns_open_question_3() {
    let p = AzureProvider::new(azure_descriptor(), http(), ApiKey::new("k".into()));
    let err = p
        .complete(simple_request())
        .await
        .expect_err("azure stub must not succeed");
    assert_open_question_3(&err, "azure");
}

#[tokio::test]
async fn cohere_stub_returns_open_question_3() {
    let p = CohereProvider::new(cohere_descriptor(), http(), ApiKey::new("k".into()));
    let err = p
        .complete(simple_request())
        .await
        .expect_err("cohere stub must not succeed");
    assert_open_question_3(&err, "cohere");
}

#[tokio::test]
async fn copilot_stub_returns_open_question_3() {
    let p = CopilotProvider::new(copilot_descriptor(), http(), ApiKey::new("k".into()));
    let err = p
        .complete(simple_request())
        .await
        .expect_err("copilot stub must not succeed");
    assert_open_question_3(&err, "copilot");
}

#[tokio::test]
async fn minimax_stub_returns_open_question_3() {
    let p = MinimaxProvider::new(minimax_descriptor(), http(), ApiKey::new("k".into()));
    let err = p
        .complete(simple_request())
        .await
        .expect_err("minimax stub must not succeed");
    assert_open_question_3(&err, "minimax");
}

#[tokio::test]
async fn stub_stream_also_returns_open_question_3() {
    let p = AzureProvider::new(azure_descriptor(), http(), ApiKey::new("k".into()));
    let err = p
        .stream(simple_request())
        .await
        .expect_err("stub stream must not succeed");
    assert_open_question_3(&err, "azure.stream");
}

#[test]
fn stub_name_returns_display_name() {
    let p = AzureProvider::new(azure_descriptor(), http(), ApiKey::new("k".into()));
    assert_eq!(p.name(), &azure_descriptor().display_name[..]);
}
