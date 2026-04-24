//! TASK-P0-B.2b (#182) — OAuth+PKCE happy-path E2E + PKCE primitive unit tests.
//!
//! Split out from the original `oauth_pkce_roundtrip.rs` as part of
//! #204 HYGIENE-MCP-FILE-SIZES. Shared mock IdP + mock MCP-SSE server
//! helpers live in `tests/common/mod.rs`.

use std::time::Duration;

use archon_mcp::oauth_pkce::{code_challenge, generate_code_verifier};
use archon_mcp::sse_oauth_transport::connect_mcp_with_oauth;

mod common;

// ---------------------------------------------------------------------------
// PKCE primitives — RFC 7636 Appendix B vector + basic invariants
// ---------------------------------------------------------------------------

#[test]
fn pkce_code_challenge_rfc7636_known_vector() {
    // RFC 7636 Appendix B: given this verifier, S256 challenge is as follows.
    // verifier  = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"
    // challenge = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
    let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    let expected = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
    assert_eq!(code_challenge(verifier), expected);
}

#[test]
fn pkce_generated_verifier_is_valid_length() {
    for _ in 0..10 {
        let v = generate_code_verifier();
        assert!(v.len() >= 43, "verifier too short: {} chars", v.len());
        assert!(v.len() <= 128, "verifier too long: {} chars", v.len());
        for c in v.chars() {
            assert!(
                c.is_ascii_alphanumeric() || "-._~".contains(c),
                "non-unreserved char in verifier: {c:?}"
            );
        }
    }
}

#[test]
fn pkce_generated_verifiers_are_unique() {
    let mut seen = std::collections::HashSet::new();
    for _ in 0..100 {
        assert!(
            seen.insert(generate_code_verifier()),
            "duplicate code_verifier"
        );
    }
}

// ---------------------------------------------------------------------------
// Happy-path E2E: full PKCE flow → OAuth-wrapped SSE → MCP roundtrip
// ---------------------------------------------------------------------------

#[tokio::test]
async fn oauth_full_flow_initialize_and_tools_list() {
    let (idp_addr, idp_server, _idp_state) = common::spawn_idp().await;
    let client = common::run_pkce_flow(idp_addr).await;

    let current = client.access_token().await;
    let (mcp_addr, mcp_server, _mcp_state) = common::spawn_mcp_with_auth(current).await;

    let sse_url = format!("http://{mcp_addr}/sse");
    let mcp_client = tokio::time::timeout(
        Duration::from_secs(10),
        connect_mcp_with_oauth(&sse_url, client, Duration::from_secs(5)),
    )
    .await
    .expect("connect within 10s")
    .expect("OAuth-wrapped connect succeeded");

    let tools = tokio::time::timeout(Duration::from_secs(5), mcp_client.list_tools())
        .await
        .expect("list_tools within 5s")
        .expect("tools/list succeeded");
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "ping");

    mcp_client.shutdown().await.expect("shutdown");
    mcp_server.abort();
    idp_server.abort();
}
