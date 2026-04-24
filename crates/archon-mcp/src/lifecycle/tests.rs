//! Unit tests for the `McpServerManager` + `backoff_delay`.
//!
//! Extracted from `lifecycle.rs` as part of #204 HYGIENE-MCP-FILE-SIZES
//! file split (zero behavioral change).

use std::collections::HashMap;

use super::*;

#[test]
fn backoff_delay_values() {
    assert_eq!(backoff_delay(1), Duration::from_secs(1));
    assert_eq!(backoff_delay(2), Duration::from_secs(2));
    assert_eq!(backoff_delay(3), Duration::from_secs(4));
    assert_eq!(backoff_delay(4), Duration::from_secs(8));
    assert_eq!(backoff_delay(5), Duration::from_secs(16));
    // After cap
    assert_eq!(backoff_delay(10), Duration::from_secs(60));
    assert_eq!(backoff_delay(100), Duration::from_secs(60));
}

#[test]
fn backoff_delay_zero() {
    // Edge case: attempt 0 should not panic
    let d = backoff_delay(0);
    assert!(d <= MAX_BACKOFF);
}

#[tokio::test]
async fn manager_new_is_empty() {
    let mgr = McpServerManager::new();
    let states = mgr.get_server_states().await;
    assert!(states.is_empty());
}

#[tokio::test]
async fn manager_start_bad_server_records_crash() {
    let mgr = McpServerManager::new();
    let config = ServerConfig {
        name: "bad-server".into(),
        command: "/nonexistent/binary".into(),
        args: vec![],
        env: HashMap::new(),
        disabled: false,
        transport: "stdio".into(),
        url: None,
        headers: None,
    };

    let errors = mgr.start_all(vec![config]).await;
    assert!(!errors.is_empty());

    let states = mgr.get_server_states().await;
    assert_eq!(states.get("bad-server"), Some(&ServerState::Crashed));
}

#[tokio::test]
async fn manager_shutdown_empty_is_ok() {
    let mgr = McpServerManager::new();
    let errors = mgr.shutdown_all().await;
    assert!(errors.is_empty());
}

#[tokio::test]
async fn manager_restart_unknown_server() {
    let mgr = McpServerManager::new();
    let result = mgr.restart_server("unknown").await;
    assert!(result.is_err());
    match result.unwrap_err() {
        McpError::ServerNotFound(name) => assert_eq!(name, "unknown"),
        other => panic!("expected ServerNotFound, got {other:?}"),
    }
}

#[tokio::test]
async fn manager_default_trait() {
    let mgr = McpServerManager::default();
    let states = mgr.get_server_states().await;
    assert!(states.is_empty());
}

/// build_mcp_tools on an empty manager returns an empty Vec.
#[tokio::test]
async fn build_mcp_tools_empty_manager_returns_empty() {
    let mgr = McpServerManager::new();
    let tools = mgr.build_mcp_tools().await;
    assert!(tools.is_empty(), "expected no tools from empty manager");
}

/// test_disable_enable_server — disable adds to set, enable removes it.
#[tokio::test]
async fn test_disable_enable_server() {
    let mgr = McpServerManager::new();

    // Disable a server that doesn't exist in the servers map yet — disabled_names
    // tracks names independently so this should still work.
    mgr.disable_server("my-server")
        .await
        .expect("disable should succeed");

    // Check it's marked disabled
    let info = mgr.get_server_info().await;
    let entry = info.iter().find(|(n, _, _)| n == "my-server");
    assert!(
        entry.is_some(),
        "disabled server should appear in get_server_info"
    );
    let (_, _, disabled) = entry.unwrap();
    assert!(
        *disabled,
        "server should be marked disabled after disable_server()"
    );

    // Enable it
    mgr.enable_server("my-server")
        .await
        .expect("enable should succeed");

    // Now it should not be in the disabled set
    let info2 = mgr.get_server_info().await;
    let entry2 = info2.iter().find(|(n, _, _)| n == "my-server");
    // After enable, if it wasn't in servers map it may not appear, but it must
    // not be disabled. If it does appear, disabled must be false.
    if let Some((_, _, d)) = entry2 {
        assert!(!d, "server should not be disabled after enable_server()");
    }
}

/// test_get_server_info_includes_disabled_flag — get_server_info returns
/// the correct disabled flag after disabling a known (crashed) server.
#[tokio::test]
async fn test_get_server_info_includes_disabled_flag() {
    let mgr = McpServerManager::new();

    // Start a server so it appears in the servers map (it will crash)
    let config = ServerConfig {
        name: "info-test-server".into(),
        command: "/nonexistent/binary".into(),
        args: vec![],
        env: HashMap::new(),
        disabled: false,
        transport: "stdio".into(),
        url: None,
        headers: None,
    };
    let _ = mgr.start_all(vec![config]).await;

    // Verify it's in the map (crashed)
    let states = mgr.get_server_states().await;
    assert_eq!(states.get("info-test-server"), Some(&ServerState::Crashed));

    // Disable it
    mgr.disable_server("info-test-server")
        .await
        .expect("disable should succeed");

    // get_server_info should show disabled=true
    let info = mgr.get_server_info().await;
    let entry = info.iter().find(|(n, _, _)| n == "info-test-server");
    assert!(entry.is_some(), "server should appear in get_server_info");
    let (_, _, disabled) = entry.unwrap();
    assert!(
        *disabled,
        "get_server_info should return disabled=true after disable_server()"
    );

    // Enable — ignore the transport error (nonexistent binary), the disabled flag is
    // cleared regardless of whether the restart succeeds.
    let _ = mgr.enable_server("info-test-server").await;
    // disabled flag should now be false even if restart fails
    let info3 = mgr.get_server_info().await;
    let entry3 = info3.iter().find(|(n, _, _)| n == "info-test-server");
    assert!(entry3.is_some(), "server should still appear after enable");
    let (_, _, d3) = entry3.unwrap();
    assert!(!d3, "disabled flag should be false after enable_server()");
}

/// build_mcp_tools skips servers that are not in Ready state.
#[tokio::test]
async fn build_mcp_tools_crashed_server_skipped() {
    let mgr = McpServerManager::new();
    // Start a server that will crash (nonexistent binary)
    let config = ServerConfig {
        name: "crashed-server".into(),
        command: "/nonexistent/binary".into(),
        args: vec![],
        env: HashMap::new(),
        disabled: false,
        transport: "stdio".into(),
        url: None,
        headers: None,
    };
    let _ = mgr.start_all(vec![config]).await;

    // Verify it's in Crashed state
    let states = mgr.get_server_states().await;
    assert_eq!(states.get("crashed-server"), Some(&ServerState::Crashed));

    // build_mcp_tools should return empty since no servers are Ready
    let tools = mgr.build_mcp_tools().await;
    assert!(
        tools.is_empty(),
        "crashed server should be skipped; got {} tools",
        tools.len()
    );
}
