//! TASK-P0-B.2b (#182) — OAuth refresh-on-401 E2E.
//!
//! Rotates the mock MCP server's accepted token mid-session to simulate
//! token expiry. The OAuth-wrapped transport's POST pump MUST:
//!   1. Observe 401 on the POST
//!   2. Call `OAuthClient::refresh`
//!   3. Retry the POST exactly once with the new bearer
//!   4. Succeed
//!
//! Split out from the original `oauth_pkce_roundtrip.rs` as part of
//! #204 HYGIENE-MCP-FILE-SIZES.

use std::time::Duration;

use archon_mcp::sse_oauth_transport::connect_mcp_with_oauth;

mod common;

#[tokio::test]
async fn oauth_401_triggers_refresh_and_retry_succeeds() {
    let (idp_addr, idp_server, _idp_state) = common::spawn_idp().await;
    let client = common::run_pkce_flow(idp_addr).await;
    let current = client.access_token().await;
    let (mcp_addr, mcp_server, mcp_state) = common::spawn_mcp_with_auth(current.clone()).await;

    // Connect + initialize successfully first (confirms the happy path works
    // with the current token).
    let sse_url = format!("http://{mcp_addr}/sse");
    let mcp_client = connect_mcp_with_oauth(&sse_url, client.clone(), Duration::from_secs(5))
        .await
        .expect("initial OAuth connect");

    // Rotate the server's accepted token BEFORE the refresh flow runs.
    // Compute what the next refresh will produce ("access-2") and prime the
    // server to accept it. This simulates a mid-session token expiry where
    // the server has already advanced to a new key.
    *mcp_state.accepted_token.lock().await = "access-2".into();

    // A fresh MCP call now sees 401 on POST. The OAuth wrapper must:
    //   a. call refresh() on the OAuthClient
    //   b. update the shared token state
    //   c. retry the POST with the new bearer
    //   d. succeed
    let tools = tokio::time::timeout(Duration::from_secs(10), mcp_client.list_tools())
        .await
        .expect("list_tools within 10s");

    let tools = tools.expect("tools/list should succeed after auto-refresh");
    assert_eq!(tools.len(), 1);

    mcp_client.shutdown().await.expect("shutdown");
    mcp_server.abort();
    idp_server.abort();
}
