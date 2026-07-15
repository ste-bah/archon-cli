use super::support::*;
use super::*;
// Server tests
// ═══════════════════════════════════════════════════════════════

#[tokio::test]
async fn server_starts_and_binds() {
    let (_dir, port_file) = temp_port_file();
    let (port, _graph, handle) = start_test_server(port_file.clone()).await;
    assert!(port > 0);
    assert!(port_file.exists(), "port file must be written");
    let contents = std::fs::read_to_string(&port_file).expect("read port file");
    assert_eq!(contents.trim().parse::<u16>().expect("valid port"), port);
    handle.abort();
}

#[tokio::test]
async fn server_ping_responds_pong() {
    let (_dir, port_file) = temp_port_file();
    let (port, _graph, handle) = start_test_server(port_file).await;

    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().expect("addr");
    let mut stream = tokio::net::TcpStream::connect(addr).await.expect("connect");

    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let req = make_request(1, "ping", serde_json::json!({}));
    let (reader, mut writer) = stream.split();
    writer.write_all(req.as_bytes()).await.expect("write");

    let mut buf_reader = BufReader::new(reader);
    let mut line = String::new();
    buf_reader.read_line(&mut line).await.expect("read");

    let resp = parse_response(&line).expect("parse");
    assert_eq!(resp.id, 1);
    assert_eq!(resp.result, Some(serde_json::json!("pong")));
    handle.abort();
}

#[tokio::test]
async fn server_store_and_recall() {
    let (_dir, port_file) = temp_port_file();
    let (port, _graph, handle) = start_test_server(port_file).await;

    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().expect("addr");
    let mut stream = tokio::net::TcpStream::connect(addr).await.expect("connect");

    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let (reader, mut writer) = stream.split();
    let mut buf_reader = BufReader::new(reader);

    // Store a memory
    let store_req = make_request(
        1,
        "store_memory",
        serde_json::json!({
            "content": "Rust is memory safe",
            "title": "rust safety",
            "memory_type": "Fact",
            "importance": 0.9,
            "tags": ["rust", "safety"],
            "source_type": "test",
            "project_path": "/tmp"
        }),
    );
    writer.write_all(store_req.as_bytes()).await.expect("write");
    let mut line = String::new();
    buf_reader.read_line(&mut line).await.expect("read");
    let resp = parse_response(&line).expect("parse store");
    assert!(resp.error.is_none(), "store should succeed");
    let stored_id = resp.result.expect("has result");

    // Recall
    let recall_req = make_request(
        2,
        "recall_memories",
        serde_json::json!({"query": "rust safety", "limit": 10}),
    );
    writer
        .write_all(recall_req.as_bytes())
        .await
        .expect("write recall");
    let mut line2 = String::new();
    buf_reader.read_line(&mut line2).await.expect("read recall");
    let resp2 = parse_response(&line2).expect("parse recall");
    assert!(resp2.error.is_none(), "recall should succeed");
    let results = resp2.result.expect("has results");
    let arr = results.as_array().expect("array");
    assert!(!arr.is_empty(), "should find at least one memory");
    assert!(
        arr[0]["content"]
            .as_str()
            .expect("content str")
            .contains("Rust")
    );

    // Verify stored_id is a string (UUID)
    assert!(stored_id.is_string(), "stored id should be a string UUID");

    handle.abort();
}

#[tokio::test]
async fn server_all_methods_dispatch() {
    let (_dir, port_file) = temp_port_file();
    let (port, _graph, handle) = start_test_server(port_file).await;

    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().expect("addr");
    let mut stream = tokio::net::TcpStream::connect(addr).await.expect("connect");

    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let (reader, mut writer) = stream.split();
    let mut buf_reader = BufReader::new(reader);

    // Helper: send + receive
    async fn send_recv(
        writer: &mut tokio::net::tcp::WriteHalf<'_>,
        buf: &mut BufReader<tokio::net::tcp::ReadHalf<'_>>,
        id: u64,
        method: &str,
        params: serde_json::Value,
    ) -> Response {
        let req = make_request(id, method, params);
        writer.write_all(req.as_bytes()).await.expect("write");
        let mut line = String::new();
        buf.read_line(&mut line).await.expect("read");
        parse_response(&line).expect("parse")
    }

    // 1. store_memory
    let resp = send_recv(
        &mut writer,
        &mut buf_reader,
        1,
        "store_memory",
        serde_json::json!({
            "content": "test content",
            "title": "test title",
            "memory_type": "Fact",
            "importance": 0.8,
            "tags": ["a"],
            "source_type": "test",
            "project_path": "/tmp"
        }),
    )
    .await;
    assert!(
        resp.error.is_none(),
        "store_memory failed: {:?}",
        resp.error
    );
    let id_a = resp.result.expect("id").as_str().expect("str").to_string();

    // 2. get_memory
    let resp = send_recv(
        &mut writer,
        &mut buf_reader,
        2,
        "get_memory",
        serde_json::json!({"id": id_a}),
    )
    .await;
    assert!(resp.error.is_none(), "get_memory failed: {:?}", resp.error);
    let mem = resp.result.expect("memory");
    assert_eq!(mem["content"].as_str().expect("c"), "test content");

    // 3. update_memory
    let resp = send_recv(
        &mut writer,
        &mut buf_reader,
        3,
        "update_memory",
        serde_json::json!({"id": id_a, "content": "updated content", "tags": ["b"]}),
    )
    .await;
    assert!(
        resp.error.is_none(),
        "update_memory failed: {:?}",
        resp.error
    );

    // Verify update
    let resp = send_recv(
        &mut writer,
        &mut buf_reader,
        4,
        "get_memory",
        serde_json::json!({"id": id_a}),
    )
    .await;
    assert_eq!(
        resp.result.expect("r")["content"].as_str().expect("c"),
        "updated content"
    );

    // 4. apply_importance_delta
    let resp = send_recv(
        &mut writer,
        &mut buf_reader,
        5,
        "apply_importance_delta",
        serde_json::json!({
            "id": id_a,
            "delta": 0.19,
            "provenance_id": "server-all-methods"
        }),
    )
    .await;
    assert!(
        resp.error.is_none(),
        "apply_importance_delta failed: {:?}",
        resp.error
    );

    // 5. Store another memory for relationship tests
    let resp = send_recv(
        &mut writer,
        &mut buf_reader,
        6,
        "store_memory",
        serde_json::json!({
            "content": "second memory",
            "title": "second",
            "memory_type": "Decision",
            "importance": 0.5,
            "tags": ["c"],
            "source_type": "test",
            "project_path": "/tmp"
        }),
    )
    .await;
    let id_b = resp.result.expect("id").as_str().expect("str").to_string();

    // 6. create_relationship
    let resp = send_recv(
        &mut writer,
        &mut buf_reader,
        7,
        "create_relationship",
        serde_json::json!({
            "from_id": id_a,
            "to_id": id_b,
            "rel_type": "RelatedTo",
            "context": "test link",
            "strength": 0.8
        }),
    )
    .await;
    assert!(
        resp.error.is_none(),
        "create_relationship failed: {:?}",
        resp.error
    );

    // 7. get_related_memories
    let resp = send_recv(
        &mut writer,
        &mut buf_reader,
        8,
        "get_related_memories",
        serde_json::json!({"id": id_a, "depth": 1}),
    )
    .await;
    assert!(
        resp.error.is_none(),
        "get_related_memories failed: {:?}",
        resp.error
    );
    let related = resp.result.expect("related");
    assert!(!related.as_array().expect("arr").is_empty());

    // 8. recall_memories
    let resp = send_recv(
        &mut writer,
        &mut buf_reader,
        9,
        "recall_memories",
        serde_json::json!({"query": "test content", "limit": 10}),
    )
    .await;
    assert!(
        resp.error.is_none(),
        "recall_memories failed: {:?}",
        resp.error
    );

    // 9. search_memories
    let resp = send_recv(
        &mut writer,
        &mut buf_reader,
        10,
        "search_memories",
        serde_json::json!({"filter": {"memory_type": "Fact"}}),
    )
    .await;
    assert!(
        resp.error.is_none(),
        "search_memories failed: {:?}",
        resp.error
    );

    // 10. list_recent
    let resp = send_recv(
        &mut writer,
        &mut buf_reader,
        11,
        "list_recent",
        serde_json::json!({"limit": 5}),
    )
    .await;
    assert!(resp.error.is_none(), "list_recent failed: {:?}", resp.error);
    let recent = resp.result.expect("recent");
    assert!(recent.as_array().expect("arr").len() >= 2);

    // 11. memory_count
    let resp = send_recv(
        &mut writer,
        &mut buf_reader,
        12,
        "memory_count",
        serde_json::json!({}),
    )
    .await;
    assert!(
        resp.error.is_none(),
        "memory_count failed: {:?}",
        resp.error
    );
    assert!(resp.result.expect("count").as_u64().expect("u64") >= 2);

    // 12. delete_memory
    let resp = send_recv(
        &mut writer,
        &mut buf_reader,
        13,
        "delete_memory",
        serde_json::json!({"id": id_b}),
    )
    .await;
    assert!(
        resp.error.is_none(),
        "delete_memory failed: {:?}",
        resp.error
    );

    // Verify deletion
    let resp = send_recv(
        &mut writer,
        &mut buf_reader,
        14,
        "get_memory",
        serde_json::json!({"id": id_b}),
    )
    .await;
    assert!(resp.error.is_some(), "deleted memory should not be found");

    // 13. clear_all
    let resp = send_recv(
        &mut writer,
        &mut buf_reader,
        15,
        "clear_all",
        serde_json::json!({}),
    )
    .await;
    assert!(resp.error.is_none(), "clear_all failed: {:?}", resp.error);

    // Verify cleared
    let resp = send_recv(
        &mut writer,
        &mut buf_reader,
        16,
        "memory_count",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(resp.result.expect("count").as_u64().expect("u64"), 0);

    handle.abort();
}

// ═══════════════════════════════════════════════════════════════
