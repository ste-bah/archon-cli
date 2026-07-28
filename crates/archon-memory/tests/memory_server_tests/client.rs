use super::support::*;
use super::*;
// Client tests
// ═══════════════════════════════════════════════════════════════

#[tokio::test]
async fn client_connects_and_pings() {
    let (_dir, port_file) = temp_port_file();
    let (port, _graph, handle) = start_test_server(port_file).await;

    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().expect("addr");
    let client = MemoryClient::connect(addr).await.expect("connect");
    client.ping().await.expect("ping");

    handle.abort();
}

#[tokio::test]
async fn client_store_memory_roundtrip() {
    let (_dir, port_file) = temp_port_file();
    let (port, _graph, handle) = start_test_server(port_file).await;

    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().expect("addr");
    let client = MemoryClient::connect(addr).await.expect("connect");

    // Store
    let result = client
        .call(
            "store_memory",
            serde_json::json!({
                "content": "client test",
                "title": "ct",
                "memory_type": "Fact",
                "importance": 0.7,
                "tags": ["test"],
                "source_type": "integration",
                "project_path": "/tmp"
            }),
        )
        .await
        .expect("store");
    let id = result.as_str().expect("id str");

    // Get
    let mem = client
        .call("get_memory", serde_json::json!({"id": id}))
        .await
        .expect("get");
    assert_eq!(mem["content"].as_str().expect("c"), "client test");

    handle.abort();
}

#[tokio::test]
async fn remote_atomic_delta_is_idempotent_by_provenance() {
    let (_dir, port_file) = temp_port_file();
    let (port, _graph, handle) = start_test_server(port_file).await;
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().expect("addr");
    let client = MemoryClient::connect(addr).await.expect("connect");
    let id = client
        .call(
            "store_memory",
            serde_json::json!({
                "content": "remote rule",
                "title": "",
                "memory_type": "Rule",
                "importance": 50.0,
                "tags": ["context", "trend:stable"],
                "source_type": "test",
                "project_path": ""
            }),
        )
        .await
        .expect("store")
        .as_str()
        .expect("id")
        .to_owned();

    for _ in 0..2 {
        client
            .call(
                "apply_importance_delta",
                serde_json::json!({
                    "id": id,
                    "delta": 10.0,
                    "provenance_id": "correction-1"
                }),
            )
            .await
            .expect("apply delta");
    }

    let stored = client
        .call("get_memory", serde_json::json!({"id": id}))
        .await
        .expect("get");
    assert_eq!(stored["importance"].as_f64(), Some(60.0));
    assert!(
        stored["tags"]
            .as_array()
            .expect("tags")
            .iter()
            .any(|tag| tag == "context")
    );
    handle.abort();
}

// ═══════════════════════════════════════════════════════════════
