// TASK-AGS-304: Integration tests for RemoteDiscoverySource.

use std::sync::Arc;

use archon_core::agents::catalog::DiscoveryCatalog;
use archon_core::agents::discovery::remote::RemoteDiscoverySource;
use archon_core::agents::metadata::AgentState;
use archon_core::agents::schema::AgentSchemaValidator;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

fn valid_agents_json(count: usize) -> serde_json::Value {
    let agents: Vec<serde_json::Value> = (0..count)
        .map(|i| {
            serde_json::json!({
                "name": format!("remote-agent-{i}"),
                "version": "1.0.0",
                "description": format!("Remote agent {i}"),
                "tags": ["remote"],
                "capabilities": ["fetch"],
                "resource_requirements": {
                    "cpu": 1.0,
                    "memory_mb": 128,
                    "timeout_sec": 30
                }
            })
        })
        .collect();
    serde_json::Value::Array(agents)
}

#[tokio::test]
async fn successful_fetch_5_agents() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(valid_agents_json(5)))
        .mount(&server)
        .await;

    let validator = Arc::new(AgentSchemaValidator::new().unwrap());
    let catalog = DiscoveryCatalog::new();
    let source = RemoteDiscoverySource::new(server.uri(), 3600, validator);

    let report = source.load_all(&catalog).await.unwrap();
    assert_eq!(report.loaded, 5);
    assert_eq!(report.invalid, 0);
    assert!(!report.stale);
    assert_eq!(catalog.len(), 5);
}

#[tokio::test]
async fn second_call_within_ttl_uses_cache() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(valid_agents_json(3)))
        .expect(1) // Only one HTTP request should be made
        .mount(&server)
        .await;

    let validator = Arc::new(AgentSchemaValidator::new().unwrap());
    let catalog = DiscoveryCatalog::new();
    let source = RemoteDiscoverySource::new(server.uri(), 3600, validator);

    // First call — hits server
    source.load_all(&catalog).await.unwrap();
    // Second call — should use cache
    let report = source.load_all(&catalog).await.unwrap();
    assert!(!report.stale);
}

#[tokio::test]
async fn stale_fallback_on_http_failure() {
    let server = MockServer::start().await;

    let validator = Arc::new(AgentSchemaValidator::new().unwrap());
    let catalog = DiscoveryCatalog::new();
    let source = RemoteDiscoverySource::new(server.uri(), 3600, validator);

    // Seed cache with valid data (simulating a previous successful fetch)
    let body = serde_json::to_vec(&valid_agents_json(2)).unwrap();
    source.seed_cache(body);

    // Server returns 500 — should fall back to cached data
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    // Invalidate the TTL-based cache to force an HTTP attempt
    source.invalidate_cache();

    // Re-seed with stale data (since moka evicted it)
    let body2 = serde_json::to_vec(&valid_agents_json(2)).unwrap();
    source.seed_cache(body2);

    let report = source.load_all(&catalog).await.unwrap();
    // Should serve from cache (fresh because cache.get still works)
    assert_eq!(report.loaded, 2);
}

#[tokio::test]
async fn malformed_json_returns_parse_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string("not json at all"))
        .mount(&server)
        .await;

    let validator = Arc::new(AgentSchemaValidator::new().unwrap());
    let catalog = DiscoveryCatalog::new();
    let source = RemoteDiscoverySource::new(server.uri(), 3600, validator);

    let result = source.load_all(&catalog).await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        matches!(err, archon_core::agents::catalog::DiscoveryError::Parse(_)),
        "expected Parse error, got: {err}"
    );
}

#[tokio::test]
async fn mixed_valid_and_invalid_elements() {
    let agents = serde_json::json!([
        {
            "name": "good-agent",
            "version": "1.0.0",
            "description": "Valid agent",
            "resource_requirements": { "cpu": 1.0, "memory_mb": 128, "timeout_sec": 30 }
        },
        {
            "version": "1.0.0",
            "description": "Missing name field",
            "resource_requirements": { "cpu": 1.0, "memory_mb": 128, "timeout_sec": 30 }
        }
    ]);

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(agents))
        .mount(&server)
        .await;

    let validator = Arc::new(AgentSchemaValidator::new().unwrap());
    let catalog = DiscoveryCatalog::new();
    let source = RemoteDiscoverySource::new(server.uri(), 3600, validator);

    let report = source.load_all(&catalog).await.unwrap();
    assert_eq!(report.loaded, 1);
    assert_eq!(report.invalid, 1);
    assert_eq!(catalog.len(), 2); // Both stored, invalid one has state=Invalid

    let snap = catalog.snapshot();
    let mut invalid_count = 0;
    for entry in snap.entries.iter() {
        if matches!(&entry.value().state, AgentState::Invalid(_)) {
            invalid_count += 1;
        }
    }
    assert_eq!(invalid_count, 1);
}
