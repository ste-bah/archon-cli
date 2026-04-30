//! Tests for TASK-CLI-228: Identity Mode Default Flip
//!
//! These tests verify clean-mode security guarantees and spoof-mode 9-layer
//! fidelity, establishing the TDD baseline before the default is changed.

use archon_llm::identity::{IdentityMode, IdentityProvider};

// ---------------------------------------------------------------------------
// Helper constructors
// ---------------------------------------------------------------------------

fn clean_provider() -> IdentityProvider {
    IdentityProvider::new(
        IdentityMode::Clean,
        "sess-test".into(),
        "dev-test".into(),
        "acct-test".into(),
    )
}

fn spoof_provider(workload: Option<String>, anti_distillation: bool) -> IdentityProvider {
    IdentityProvider::new(
        IdentityMode::Spoof {
            version: "2.1.89".into(),
            entrypoint: "cli".into(),
            betas: vec![
                "claude-code-20250219".into(),
                "oauth-2025-04-20".into(),
                "interleaved-thinking-2025-05-14".into(),
                "prompt-caching-scope-2026-01-05".into(),
            ],
            workload,
            anti_distillation,
        },
        "sess-spoof".into(),
        "dev-spoof".into(),
        "acct-spoof".into(),
    )
}

// =========================================================================
// Clean mode security
// =========================================================================

#[test]
fn clean_user_agent_is_archon() {
    let headers = clean_provider().request_headers("req-1");
    let ua = headers
        .get("User-Agent")
        .expect("User-Agent must be present");
    assert!(
        ua.starts_with("archon-cli/"),
        "clean UA should start with archon-cli/, got: {ua}"
    );
}

#[test]
fn clean_x_app_is_archon() {
    let headers = clean_provider().request_headers("req-2");
    assert_eq!(
        headers.get("x-app").map(String::as_str),
        Some("archon"),
        "clean mode x-app must be 'archon'"
    );
}

#[test]
fn clean_no_session_id_header() {
    let headers = clean_provider().request_headers("req-3");
    assert!(
        !headers.contains_key("X-Claude-Code-Session-Id"),
        "clean mode must not leak X-Claude-Code-Session-Id"
    );
}

#[test]
fn clean_no_request_id_header() {
    let headers = clean_provider().request_headers("req-4");
    assert!(
        !headers.contains_key("x-client-request-id"),
        "clean mode must not leak x-client-request-id"
    );
}

#[test]
fn clean_no_beta_header() {
    let headers = clean_provider().request_headers("req-5");
    assert!(
        !headers.contains_key("anthropic-beta"),
        "clean mode must not include anthropic-beta header"
    );
}

#[test]
fn clean_metadata_empty() {
    let meta = clean_provider().metadata();
    assert!(
        meta.as_object().map(|o| o.is_empty()).unwrap_or(false),
        "clean mode metadata must be empty object, got: {meta}"
    );
}

#[test]
fn clean_no_billing_header() {
    let bh = clean_provider().billing_header("any user message");
    assert!(bh.is_none(), "clean mode must not produce a billing header");
}

#[test]
fn clean_system_blocks_no_cache_scope() {
    let blocks = clean_provider().system_prompt_blocks("msg", "static", "dynamic");
    for (i, block) in blocks.iter().enumerate() {
        let scope = block.pointer("/cache_control/scope");
        assert!(
            scope.is_none(),
            "clean mode block {i} must not have cache_control.scope, got: {block}"
        );
    }
}

#[test]
fn clean_only_four_headers() {
    let headers = clean_provider().request_headers("req-6");
    assert_eq!(
        headers.len(),
        4,
        "clean mode should produce exactly 4 headers (User-Agent, x-app, anthropic-version, content-type), got {}: {:?}",
        headers.len(),
        headers.keys().collect::<Vec<_>>()
    );
}

// =========================================================================
// Spoof mode -- 9 layers
// =========================================================================

#[test]
fn spoof_layer1_user_agent() {
    let headers = spoof_provider(None, false).request_headers("req-s1");
    assert_eq!(
        headers.get("User-Agent").map(String::as_str),
        Some("claude-cli/2.1.89 (external, cli)"),
        "spoof layer 1: User-Agent must mimic Claude Code"
    );
}

#[test]
fn spoof_layer2_fingerprint() {
    let provider = spoof_provider(None, false);
    let bh = provider
        .billing_header("sample user message for fingerprint testing")
        .expect("spoof must produce billing header");
    let version_prefix = "cc_version=2.1.89.";
    let start = bh
        .find(version_prefix)
        .expect("billing header must contain cc_version=2.1.89.");
    let after_prefix = &bh[start + version_prefix.len()..];
    let fp: String = after_prefix
        .chars()
        .take_while(|c| c.is_ascii_hexdigit())
        .collect();
    assert_eq!(
        fp.len(),
        3,
        "spoof layer 2: fingerprint must be 3 hex chars, got: '{fp}'"
    );
}

#[test]
fn spoof_layer3_betas() {
    let headers = spoof_provider(None, false).request_headers("req-s3");
    let beta = headers
        .get("anthropic-beta")
        .expect("spoof must have anthropic-beta header");
    assert!(
        beta.contains("claude-code-20250219"),
        "spoof layer 3: anthropic-beta must contain betas, got: {beta}"
    );
}

#[test]
fn spoof_layer4_metadata() {
    let provider = spoof_provider(None, false);
    let meta = provider.metadata();
    let user_id_str = meta["user_id"]
        .as_str()
        .expect("spoof metadata must have user_id string");

    let inner: serde_json::Value =
        serde_json::from_str(user_id_str).expect("user_id must be valid JSON");

    assert!(
        inner.get("device_id").is_some(),
        "spoof layer 4: metadata must contain device_id"
    );
    assert!(
        inner.get("account_uuid").is_some(),
        "spoof layer 4: metadata must contain account_uuid"
    );
    assert!(
        inner.get("session_id").is_some(),
        "spoof layer 4: metadata must contain session_id"
    );
}

#[test]
fn spoof_layer5_billing() {
    let provider = spoof_provider(None, false);
    let bh = provider
        .billing_header("hello world test message")
        .expect("spoof must produce billing header");
    assert!(
        bh.contains("x-anthropic-billing-header:"),
        "spoof layer 5: must contain billing header prefix"
    );
    assert!(
        bh.contains("cc_version="),
        "spoof layer 5: must contain cc_version="
    );
    assert!(
        bh.contains("cc_entrypoint="),
        "spoof layer 5: must contain cc_entrypoint="
    );
}

#[test]
fn spoof_layer6_identity_prefix() {
    let provider = spoof_provider(None, false);
    let blocks = provider.system_prompt_blocks("msg", "static content", "dynamic content");
    assert!(
        blocks.len() >= 2,
        "spoof must produce at least 2 system blocks, got {}",
        blocks.len()
    );
    let identity_block_text = blocks[1]["text"]
        .as_str()
        .expect("block[1] must have text field");
    assert!(
        identity_block_text.contains("You are Claude Code"),
        "spoof layer 6: identity prefix must contain 'You are Claude Code', got: {identity_block_text}"
    );
}

#[test]
fn spoof_layer7_cache_scopes() {
    let provider = spoof_provider(None, false);
    let blocks = provider.system_prompt_blocks("msg", "static", "dynamic");

    assert_eq!(
        blocks[0]["cache_control"]["type"].as_str(),
        Some("ephemeral"),
        "spoof layer 7: block 0 must be ephemeral"
    );

    assert_eq!(
        blocks[1]["cache_control"]["scope"].as_str(),
        Some("org"),
        "spoof layer 7: block 1 must have scope=org"
    );

    assert_eq!(
        blocks[2]["cache_control"]["scope"].as_str(),
        Some("global"),
        "spoof layer 7: block 2 must have scope=global"
    );
}

#[test]
fn spoof_layer8_workload() {
    let provider = spoof_provider(Some("cron".into()), false);
    let bh = provider
        .billing_header("workload test message")
        .expect("spoof with workload must produce billing header");
    assert!(
        bh.contains("cc_workload=cron"),
        "spoof layer 8: billing header must contain cc_workload=cron, got: {bh}"
    );
}

#[test]
fn spoof_layer9_anti_distillation() {
    let provider = spoof_provider(None, true);
    match &provider.mode {
        IdentityMode::Spoof {
            anti_distillation, ..
        } => {
            assert!(
                *anti_distillation,
                "spoof layer 9: anti_distillation must be true when set"
            );
        }
        other => panic!("expected Spoof mode, got: {other:?}"),
    }
}

// =========================================================================
// Cross-mode comparison
// =========================================================================

#[test]
fn spoof_headers_absent_from_clean() {
    let clean_headers = clean_provider().request_headers("req-clean");
    let spoof_headers = spoof_provider(None, false).request_headers("req-spoof");

    let spoof_only_keys = [
        "X-Claude-Code-Session-Id",
        "x-client-request-id",
        "anthropic-beta",
    ];

    for key in &spoof_only_keys {
        assert!(
            spoof_headers.contains_key(*key),
            "spoof mode should have header '{key}' (test sanity check)"
        );
        assert!(
            !clean_headers.contains_key(*key),
            "clean mode must NOT have spoof-specific header '{key}'"
        );
    }
}
