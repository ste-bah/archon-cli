//! TASK-AGS-505: RemoteAgentPattern — HTTP federation for remote agent invocation.
//!
//! Implements the `Pattern` trait for invoking remote agents over HTTP.
//! Supports pluggable service discovery via the `DiscoveryResolver` trait,
//! with a `StaticResolver` provided out of the box.
//!
//! Only HTTP transport is implemented (POST to `{endpoint}/invoke`).
//! gRPC is out of scope for this task.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use url::Url;

use super::{Pattern, PatternCtx, PatternError, PatternKind, PatternRegistry, RemoteAgentConfig};

// ---------------------------------------------------------------------------
// DiscoveryResolver — pluggable service resolution
// ---------------------------------------------------------------------------

/// Resolves a logical service name to a concrete URL.
///
/// Implementations may query Consul, etcd, DNS SRV, or use a static map.
#[async_trait]
pub trait DiscoveryResolver: Send + Sync {
    /// Resolve `service` to a base URL suitable for HTTP invocation.
    async fn resolve(&self, service: &str) -> Result<Url, PatternError>;
}

// ---------------------------------------------------------------------------
// StaticResolver
// ---------------------------------------------------------------------------

/// In-memory resolver backed by a `HashMap<String, Url>`.
///
/// Useful for testing and for deployments where endpoints are known at
/// configuration time.
pub struct StaticResolver {
    map: HashMap<String, Url>,
}

impl StaticResolver {
    pub fn new(map: HashMap<String, Url>) -> Self {
        Self { map }
    }
}

#[async_trait]
impl DiscoveryResolver for StaticResolver {
    async fn resolve(&self, service: &str) -> Result<Url, PatternError> {
        self.map.get(service).cloned().ok_or_else(|| {
            PatternError::RemoteUnreachable {
                url: service.to_string(),
                cause: format!("service '{service}' not found in static resolver"),
            }
        })
    }
}

// ---------------------------------------------------------------------------
// RemoteAgentPattern
// ---------------------------------------------------------------------------

/// HTTP-based remote agent invocation pattern.
///
/// Holds a shared `reqwest::Client` for connection pooling and an optional
/// `DiscoveryResolver` for dynamic endpoint resolution.
pub struct RemoteAgentPattern {
    http_client: reqwest::Client,
    config: RemoteAgentConfig,
    resolver: Option<Arc<dyn DiscoveryResolver>>,
    timeout: Duration,
}

impl RemoteAgentPattern {
    /// Create a new `RemoteAgentPattern`.
    ///
    /// * `http_client` — shared reqwest client (connection pooling).
    /// * `config` — parsed `RemoteAgentConfig` from the pattern spec.
    /// * `resolver` — optional discovery resolver for dynamic endpoints.
    /// * `timeout` — per-invocation timeout from the `PatternSpec`.
    pub fn new(
        http_client: reqwest::Client,
        config: RemoteAgentConfig,
        resolver: Option<Arc<dyn DiscoveryResolver>>,
        timeout: Duration,
    ) -> Self {
        Self {
            http_client,
            config,
            resolver,
            timeout,
        }
    }

    /// Determine the endpoint URL: resolve via discovery if a resolver is
    /// present and `service_discovery` is configured, otherwise fall back to
    /// the direct `config.endpoint`.
    async fn resolve_endpoint(&self) -> Result<Url, PatternError> {
        if let (Some(resolver), Some(discovery)) =
            (&self.resolver, &self.config.service_discovery)
        {
            // Use the discovery backend's identifying string as the service key.
            let service_key = match discovery {
                super::DiscoveryBackend::Consul { address } => address.clone(),
                super::DiscoveryBackend::Etcd { endpoints } => {
                    endpoints.first().cloned().unwrap_or_default()
                }
                super::DiscoveryBackend::DnsSrv { domain } => domain.clone(),
            };
            resolver.resolve(&service_key).await
        } else {
            Ok(self.config.endpoint.clone())
        }
    }

    /// Execute the HTTP POST to `{endpoint}/invoke` and map errors.
    async fn http_invoke(&self, endpoint: Url, input: Value) -> Result<Value, PatternError> {
        let invoke_url = format!("{}invoke", endpoint.as_str().trim_end_matches('/').to_owned() + "/");

        let mut request = self
            .http_client
            .post(&invoke_url)
            .json(&json!({ "input": input }));

        // Apply bearer auth if configured.
        if let super::AuthConfig::Bearer(ref token) = self.config.auth {
            request = request.bearer_auth(token);
        }

        let response = request.send().await.map_err(|e| {
            if e.is_timeout() {
                PatternError::Timeout
            } else {
                PatternError::RemoteUnreachable {
                    url: invoke_url.clone(),
                    cause: e.to_string(),
                }
            }
        })?;

        let status = response.status();

        if status.is_server_error() {
            return Err(PatternError::RemoteUnreachable {
                url: invoke_url,
                cause: format!("HTTP {status}"),
            });
        }

        if status.is_client_error() {
            let body = response.text().await.unwrap_or_default();
            return Err(PatternError::Execution(format!(
                "remote 4xx: HTTP {status}: {body}"
            )));
        }

        // Parse the JSON response — expecting `{"output": <value>}`.
        let body: Value = response.json().await.map_err(|e| {
            PatternError::Execution(format!("failed to parse remote response: {e}"))
        })?;

        // Extract the "output" field; if absent, return the full body.
        Ok(body.get("output").cloned().unwrap_or(body))
    }
}

#[async_trait]
impl Pattern for RemoteAgentPattern {
    fn kind(&self) -> PatternKind {
        PatternKind::Remote
    }

    async fn execute(
        &self,
        input: Value,
        _ctx: PatternCtx,
    ) -> Result<Value, PatternError> {
        let endpoint = self.resolve_endpoint().await?;

        // Wrap the entire call in a timeout.
        match tokio::time::timeout(self.timeout, self.http_invoke(endpoint, input)).await {
            Ok(result) => result,
            Err(_elapsed) => Err(PatternError::Timeout),
        }
    }
}

/// Register a `RemoteAgentPattern` into the registry under name "remote".
pub fn register(
    reg: &PatternRegistry,
    http_client: reqwest::Client,
    config: RemoteAgentConfig,
    resolver: Option<Arc<dyn DiscoveryResolver>>,
    timeout: Duration,
) {
    reg.register(
        "remote",
        Arc::new(RemoteAgentPattern::new(http_client, config, resolver, timeout)),
    );
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::time::Duration;

    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use crate::patterns::{
        AuthConfig, PatternRegistry, RemoteAgentConfig, RemoteTransport, TaskServiceHandle,
    };

    /// Build a `RemoteAgentConfig` pointing at the given base URL.
    fn make_config(base_url: &str) -> RemoteAgentConfig {
        RemoteAgentConfig {
            transport: RemoteTransport::Http,
            endpoint: Url::parse(base_url).unwrap(),
            auth: AuthConfig::None,
            service_discovery: None,
        }
    }

    /// Build a `PatternCtx` with a dummy task service.
    fn make_ctx() -> PatternCtx {
        struct DummyTaskService;

        #[async_trait]
        impl TaskServiceHandle for DummyTaskService {
            async fn submit(
                &self,
                _agent: &str,
                input: Value,
            ) -> Result<Value, PatternError> {
                Ok(input)
            }
        }

        PatternCtx {
            task_service: Arc::new(DummyTaskService),
            registry: Arc::new(PatternRegistry::new()),
            trace_id: "test".into(),
            deadline: None,
        }
    }

    // ----- test_remote_http_success_returns_output -----

    #[tokio::test]
    async fn test_remote_http_success_returns_output() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/invoke"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"output": {"ok": true}})),
            )
            .mount(&server)
            .await;

        let config = make_config(&server.uri());
        let pattern = RemoteAgentPattern::new(
            reqwest::Client::new(),
            config,
            None,
            Duration::from_secs(5),
        );

        let result = pattern
            .execute(json!({"question": "hello"}), make_ctx())
            .await
            .unwrap();

        assert_eq!(result, json!({"ok": true}));
    }

    // ----- test_remote_http_5xx_returns_unreachable -----

    #[tokio::test]
    async fn test_remote_http_5xx_returns_unreachable() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/invoke"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;

        let config = make_config(&server.uri());
        let pattern = RemoteAgentPattern::new(
            reqwest::Client::new(),
            config,
            None,
            Duration::from_secs(5),
        );

        let err = pattern
            .execute(json!({}), make_ctx())
            .await
            .unwrap_err();

        match err {
            PatternError::RemoteUnreachable { url, cause } => {
                assert!(url.contains("/invoke"), "url should mention /invoke: {url}");
                assert!(
                    cause.contains("503"),
                    "cause should mention status code: {cause}"
                );
            }
            other => panic!("expected RemoteUnreachable, got: {other}"),
        }
    }

    // ----- test_remote_http_4xx_returns_execution_error -----

    #[tokio::test]
    async fn test_remote_http_4xx_returns_execution_error() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/invoke"))
            .respond_with(
                ResponseTemplate::new(400).set_body_string("bad request body"),
            )
            .mount(&server)
            .await;

        let config = make_config(&server.uri());
        let pattern = RemoteAgentPattern::new(
            reqwest::Client::new(),
            config,
            None,
            Duration::from_secs(5),
        );

        let err = pattern
            .execute(json!({}), make_ctx())
            .await
            .unwrap_err();

        match err {
            PatternError::Execution(msg) => {
                assert!(
                    msg.contains("remote 4xx"),
                    "message should contain 'remote 4xx': {msg}"
                );
                assert!(msg.contains("400"), "message should contain status: {msg}");
            }
            other => panic!("expected Execution, got: {other}"),
        }
    }

    // ----- test_remote_timeout_returns_timeout_variant -----

    #[tokio::test]
    async fn test_remote_timeout_returns_timeout_variant() {
        let server = MockServer::start().await;

        // Respond after 5 seconds — but the pattern timeout is only 50ms.
        Mock::given(method("POST"))
            .and(path("/invoke"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"output": "slow"}))
                    .set_delay(Duration::from_secs(5)),
            )
            .mount(&server)
            .await;

        let config = make_config(&server.uri());
        let pattern = RemoteAgentPattern::new(
            reqwest::Client::new(),
            config,
            None,
            Duration::from_millis(50), // very short timeout
        );

        let err = pattern
            .execute(json!({}), make_ctx())
            .await
            .unwrap_err();

        assert!(
            matches!(err, PatternError::Timeout),
            "expected Timeout, got: {err}"
        );
    }

    // ----- test_static_resolver_round_trip -----

    #[tokio::test]
    async fn test_static_resolver_round_trip() {
        let expected_url = Url::parse("http://agent-a.local:8080").unwrap();
        let mut map = HashMap::new();
        map.insert("agent-a".to_string(), expected_url.clone());

        let resolver = StaticResolver::new(map);

        // Successful lookup.
        let resolved = resolver.resolve("agent-a").await.unwrap();
        assert_eq!(resolved, expected_url);

        // Missing service.
        let err = resolver.resolve("agent-b").await.unwrap_err();
        assert!(
            matches!(err, PatternError::RemoteUnreachable { .. }),
            "missing service should be RemoteUnreachable: {err}"
        );
    }

    // ----- test_remote_http_unreachable_returns_remote_unreachable -----

    #[tokio::test]
    async fn test_remote_http_unreachable_returns_remote_unreachable() {
        // Point at a port that is almost certainly not listening.
        let config = make_config("http://127.0.0.1:1");
        let pattern = RemoteAgentPattern::new(
            reqwest::Client::new(),
            config,
            None,
            Duration::from_secs(5),
        );

        let err = pattern
            .execute(json!({}), make_ctx())
            .await
            .unwrap_err();

        assert!(
            matches!(err, PatternError::RemoteUnreachable { .. }),
            "connection refused should be RemoteUnreachable: {err}"
        );
    }

    // ----- test_register_and_resolve -----

    #[test]
    fn test_register_and_resolve() {
        let reg = PatternRegistry::new();
        let config = make_config("http://example.com");

        register(
            &reg,
            reqwest::Client::new(),
            config,
            None,
            Duration::from_secs(30),
        );

        let resolved = reg.resolve("remote");
        assert!(resolved.is_some());
        assert!(matches!(resolved.unwrap().kind(), PatternKind::Remote));
    }

    // ----- test_pattern_kind -----

    #[test]
    fn test_pattern_kind() {
        let config = make_config("http://example.com");
        let pattern = RemoteAgentPattern::new(
            reqwest::Client::new(),
            config,
            None,
            Duration::from_secs(30),
        );
        assert!(matches!(pattern.kind(), PatternKind::Remote));
    }
}
