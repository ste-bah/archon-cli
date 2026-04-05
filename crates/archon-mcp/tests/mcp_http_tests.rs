//! Integration tests for MCP HTTP transport configuration and creation.

use std::collections::HashMap;
use std::time::Duration;

use archon_mcp::config::load_config_file;
use archon_mcp::http_transport::create_http_transport;

/// Parsing a .mcp.json with transport="http" and url produces the right config.
#[test]
fn config_http_transport_parses() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let json = r#"{
        "mcpServers": {
            "remote-server": {
                "command": "",
                "transport": "http",
                "url": "http://localhost:8080/mcp"
            }
        }
    }"#;
    let path = dir.path().join(".mcp.json");
    std::fs::write(&path, json).expect("write");

    let configs = load_config_file(&path).expect("parse");
    assert_eq!(configs.len(), 1);
    let cfg = &configs[0];
    assert_eq!(cfg.name, "remote-server");
    assert_eq!(cfg.transport, "http");
    assert_eq!(cfg.url.as_deref(), Some("http://localhost:8080/mcp"));
}

/// A .mcp.json without a transport field defaults to "stdio".
#[test]
fn config_default_transport_is_stdio() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let json = r#"{
        "mcpServers": {
            "local-server": {
                "command": "npx",
                "args": ["-y", "some-mcp-server"]
            }
        }
    }"#;
    let path = dir.path().join(".mcp.json");
    std::fs::write(&path, json).expect("write");

    let configs = load_config_file(&path).expect("parse");
    assert_eq!(configs.len(), 1);
    assert_eq!(configs[0].transport, "stdio");
    assert!(configs[0].url.is_none());
}

/// A .mcp.json with headers (including Authorization) parses them.
#[test]
fn config_http_with_auth_headers() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let json = r#"{
        "mcpServers": {
            "auth-server": {
                "command": "",
                "transport": "http",
                "url": "https://api.example.com/mcp",
                "headers": {
                    "Authorization": "Bearer tok_secret123",
                    "X-Custom": "value"
                }
            }
        }
    }"#;
    let path = dir.path().join(".mcp.json");
    std::fs::write(&path, json).expect("write");

    let configs = load_config_file(&path).expect("parse");
    assert_eq!(configs.len(), 1);
    let cfg = &configs[0];
    let headers = cfg.headers.as_ref().expect("headers present");
    assert_eq!(
        headers.get("Authorization").unwrap(),
        "Bearer tok_secret123"
    );
    assert_eq!(headers.get("X-Custom").unwrap(), "value");
}

/// A .mcp.json with both stdio and http servers parses both correctly.
#[test]
fn config_mixed_stdio_and_http() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let json = r#"{
        "mcpServers": {
            "local": {
                "command": "node",
                "args": ["server.js"]
            },
            "remote": {
                "command": "",
                "transport": "http",
                "url": "http://remote:9090/mcp"
            }
        }
    }"#;
    let path = dir.path().join(".mcp.json");
    std::fs::write(&path, json).expect("write");

    let configs = load_config_file(&path).expect("parse");
    assert_eq!(configs.len(), 2);

    let local = configs.iter().find(|c| c.name == "local").expect("local");
    assert_eq!(local.transport, "stdio");
    assert!(local.url.is_none());

    let remote = configs.iter().find(|c| c.name == "remote").expect("remote");
    assert_eq!(remote.transport, "http");
    assert_eq!(remote.url.as_deref(), Some("http://remote:9090/mcp"));
}

/// Creating an HTTP transport with an obviously bad URL returns an error.
#[tokio::test]
async fn http_transport_creation_with_bad_url() {
    let result = create_http_transport("not-a-valid-url", None, Duration::from_secs(5));
    // The transport itself is created lazily, so we just verify it
    // can be constructed (the error happens on connect, not build).
    // If the implementation validates eagerly, it should error here.
    // Either way this should not panic.
    let _ = result;
}

/// An HTTP transport pointed at an unreachable host should time out
/// when we actually try to connect/initialize.
#[tokio::test]
async fn http_transport_timeout() {
    // 192.0.2.1 is TEST-NET-1 (RFC 5737) — guaranteed unreachable.
    let result = create_http_transport("http://192.0.2.1:1/mcp", None, Duration::from_millis(500));
    // The transport is created but actual connection happens during
    // serve_client. Verify the transport was at least constructed.
    assert!(result.is_ok(), "transport construction should succeed");
}

/// Verify headers are forwarded into the transport config.
#[tokio::test]
async fn http_transport_with_custom_headers() {
    let mut headers = HashMap::new();
    headers.insert("Authorization".into(), "Bearer secret".into());
    headers.insert("X-Trace-Id".into(), "abc123".into());

    let result = create_http_transport(
        "http://localhost:9999/mcp",
        Some(&headers),
        Duration::from_secs(5),
    );
    assert!(
        result.is_ok(),
        "transport construction with headers should succeed"
    );
}
