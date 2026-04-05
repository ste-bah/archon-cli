//! HTTP Streamable transport layer for MCP servers.
//!
//! Uses `rmcp`'s `StreamableHttpClientTransport` with `reqwest` to connect
//! to MCP servers over HTTP/SSE instead of stdio child processes.

use std::collections::HashMap;
use std::time::Duration;

use http::{HeaderName, HeaderValue};
use rmcp::transport::streamable_http_client::{
    StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
};

use crate::types::McpError;

/// The concrete transport type returned by [`create_http_transport`].
pub type HttpTransport = StreamableHttpClientTransport<reqwest::Client>;

/// Create a Streamable HTTP client transport for an MCP server.
///
/// Builds a `reqwest::Client` with the given `connect_timeout` and optional
/// custom headers (e.g. `Authorization`), then wraps it in an rmcp
/// `StreamableHttpClientTransport`.
///
/// The returned transport is lazy — the actual HTTP connection is established
/// when `serve_client` is called, not during construction.
pub fn create_http_transport(
    url: &str,
    headers: Option<&HashMap<String, String>>,
    connect_timeout: Duration,
) -> Result<HttpTransport, McpError> {
    let reqwest_client = reqwest::Client::builder()
        .connect_timeout(connect_timeout)
        .build()
        .map_err(|e| McpError::Transport(format!("failed to build HTTP client: {e}")))?;

    let mut custom_headers: HashMap<HeaderName, HeaderValue> = HashMap::new();

    if let Some(hdrs) = headers {
        for (name, value) in hdrs {
            let header_name: HeaderName = name
                .parse()
                .map_err(|e| McpError::Transport(format!("invalid header name '{name}': {e}")))?;
            let header_value: HeaderValue = value.parse().map_err(|e| {
                McpError::Transport(format!("invalid header value for '{name}': {e}"))
            })?;
            custom_headers.insert(header_name, header_value);
        }
    }

    let config = StreamableHttpClientTransportConfig::with_uri(url).custom_headers(custom_headers);

    let transport = StreamableHttpClientTransport::with_client(reqwest_client, config);
    Ok(transport)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn create_transport_with_valid_url() {
        let result =
            create_http_transport("http://localhost:8080/mcp", None, Duration::from_secs(5));
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn create_transport_with_headers() {
        let mut headers = HashMap::new();
        headers.insert("Authorization".into(), "Bearer token".into());
        headers.insert("X-Api-Key".into(), "key123".into());

        let result = create_http_transport(
            "http://localhost:8080/mcp",
            Some(&headers),
            Duration::from_secs(5),
        );
        assert!(result.is_ok());
    }

    #[test]
    fn create_transport_with_invalid_header_name() {
        let mut headers = HashMap::new();
        headers.insert("Invalid Header\nName".into(), "value".into());

        let result = create_http_transport(
            "http://localhost:8080/mcp",
            Some(&headers),
            Duration::from_secs(5),
        );
        assert!(result.is_err());
        match result {
            Err(McpError::Transport(msg)) => {
                assert!(msg.contains("invalid header name"));
            }
            Err(other) => panic!("expected Transport error, got {other}"),
            Ok(_) => panic!("expected error"),
        }
    }

    #[test]
    fn create_transport_with_invalid_header_value() {
        let mut headers = HashMap::new();
        headers.insert("X-Bad".into(), "value\0with\0nulls".into());

        let result = create_http_transport(
            "http://localhost:8080/mcp",
            Some(&headers),
            Duration::from_secs(5),
        );
        assert!(result.is_err());
        match result {
            Err(McpError::Transport(msg)) => {
                assert!(msg.contains("invalid header value"));
            }
            Err(other) => panic!("expected Transport error, got {other}"),
            Ok(_) => panic!("expected error"),
        }
    }
}
