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

/// The remote answer to `embedding_neighbours` must equal the direct one.
///
/// This was the last memory operation missing from the RPC surface, and the
/// gap was invisible: the remote side answered with an empty vector, which the
/// semantic dedup pass read as "no duplicates". Since every Archon process
/// after the first reads memory over TCP, the pass was skipped far more often
/// than it ran, and reported as clean each time.
///
/// Ids AND distances are compared, not merely "something came back". A merge
/// threshold set from measured cosine distances is worthless if the wire
/// rounds, reorders, or inverts them.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remote_embedding_neighbours_matches_direct() {
    let (_dir, port_file) = temp_port_file();
    let (port, graph, handle) = start_test_server(port_file).await;
    archon_memory::vector_search::init_embedding_schema(graph.db(), 4).expect("embedding schema");

    let store = |content: &str, importance: f64| {
        graph
            .store_memory(content, "", MemoryType::Fact, importance, &[], "test", "")
            .expect("store")
    };
    let anchor = store("deploy to eu-west-2", 0.9);
    let paraphrase = store("target the eu-west-2 region", 0.4);
    let unrelated = store("python is good for data science", 0.5);

    // Hand-built so the geometry is exact rather than model-dependent.
    let put = |id: &str, v: [f32; 4]| {
        archon_memory::vector_search::store_embedding(graph.db(), id, &v, "test", 4)
            .expect("embedding")
    };
    put(&anchor, [1.0, 0.0, 0.0, 0.0]);
    put(&paraphrase, [0.99, 0.09, 0.0, 0.0]);
    put(&unrelated, [0.0, 1.0, 0.0, 0.0]);

    let direct = MemoryTrait::embedding_neighbours(graph.as_ref(), &anchor, 8)
        .expect("direct neighbour search")
        .expect("a store with a live index reports available");
    assert!(
        direct.iter().any(|(id, _)| *id == paraphrase),
        "the fixture is wrong if the direct query finds no paraphrase: {direct:?}"
    );

    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().expect("addr");
    let client = MemoryClient::connect(addr).await.expect("connect");
    // `MemoryAccess::Remote` holds nothing but the socket, so a matching answer
    // can only have come back over TCP.
    let access = MemoryAccess::Remote(client);
    let remote = access
        .embedding_neighbours(&anchor, 8)
        .expect("remote neighbour search")
        .expect("the server reports its index available");

    assert_eq!(
        remote, direct,
        "remote neighbours must match direct, ids and distances alike"
    );

    handle.abort();
}

/// A server with no vector index answers "unavailable", not "empty".
///
/// The `null` has to survive the wire intact. Were it flattened to an empty
/// list anywhere along the way, the caller would be back to reading an absent
/// pass as a clean store -- the exact bug.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remote_embedding_neighbours_reports_an_unindexed_server_as_unavailable() {
    let (_dir, port_file) = temp_port_file();
    let (port, graph, handle) = start_test_server(port_file).await;
    let id = graph
        .store_memory("no index here", "", MemoryType::Fact, 0.5, &[], "test", "")
        .expect("store");

    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().expect("addr");
    let client = MemoryClient::connect(addr).await.expect("connect");
    let access = MemoryAccess::Remote(client);

    assert_eq!(
        access
            .embedding_neighbours(&id, 8)
            .expect("remote neighbour search"),
        None,
        "an unindexed server must report unavailable, not an empty neighbour list"
    );

    handle.abort();
}

// ═══════════════════════════════════════════════════════════════
