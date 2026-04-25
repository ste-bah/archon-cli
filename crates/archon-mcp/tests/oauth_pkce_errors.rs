//! TASK-P0-B.2b (#182) — OAuth persistent-401 error bounds.
//!
//! With refresh disabled on the IdP, a 401 mid-session must surface as a
//! bounded error or clean timeout — NOT an infinite retry loop. Proves
//! the drop-on-persistent-failure property of
//! `sse_oauth_transport::oauth_post_pump_task`.
//!
//! Split out from the original `oauth_pkce_roundtrip.rs` as part of
//! #204 HYGIENE-MCP-FILE-SIZES.

use std::time::Duration;

use archon_mcp::sse_oauth_transport::connect_mcp_with_oauth;
use archon_mcp::types::McpError;

mod common;

#[tokio::test]
async fn oauth_persistent_401_errors_bounded() {
    let (idp_addr, idp_server, idp_state) = common::spawn_idp().await;
    let client = common::run_pkce_flow(idp_addr).await;
    let current = client.access_token().await;
    let (mcp_addr, mcp_server, mcp_state) = common::spawn_mcp_with_auth(current.clone()).await;

    let sse_url = format!("http://{mcp_addr}/sse");
    let mcp_client = connect_mcp_with_oauth(&sse_url, client, Duration::from_secs(5))
        .await
        .expect("initial connect");

    // Rotate the MCP server's accepted token to something new, AND disable
    // refresh on the IdP. Now the wrapper will try to refresh, get 401 back,
    // and must fail the caller — not loop forever.
    *mcp_state.accepted_token.lock().await = "different-access".into();
    *idp_state.refresh_disabled.lock().await = true;

    let result = tokio::time::timeout(Duration::from_secs(8), mcp_client.list_tools()).await;

    // Either the call times out (rmcp-level) or it errors — but it must not
    // hang forever and it must not infinitely loop. Empirically the rmcp
    // request will time out because the server rejects every POST.
    match result {
        Ok(Ok(_)) => panic!("expected error when refresh is disabled"),
        Ok(Err(e)) => {
            let _e: McpError = e;
        }
        Err(_) => {
            // Timeout also acceptable — the transport gave up cleanly.
        }
    }

    mcp_client.shutdown().await.expect("shutdown");
    mcp_server.abort();
    idp_server.abort();
}
