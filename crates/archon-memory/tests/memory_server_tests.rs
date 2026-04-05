//! Integration tests for the singleton memory server pattern.
//!
//! Covers protocol serialization, TCP server dispatch, client
//! round-trips, MemoryTrait implementations, and the access factory.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use archon_memory::MemoryGraph;
use archon_memory::access::{MemoryAccess, MemoryTrait, open_memory};
use archon_memory::client::MemoryClient;
use archon_memory::protocol::{Request, Response, make_request, parse_response};
use archon_memory::server::MemoryServer;
use archon_memory::types::{MemoryType, RelType, SearchFilter};

// ── helpers ────────────────────────────────────────────────────

/// Spin up an in-memory graph behind a TCP server, returning (port, graph, join-handle).
async fn start_test_server(
    port_file: PathBuf,
) -> (u16, Arc<MemoryGraph>, tokio::task::JoinHandle<()>) {
    let graph = Arc::new(MemoryGraph::in_memory().expect("in-memory graph"));
    let (port, handle) = MemoryServer::start(Arc::clone(&graph), port_file)
        .await
        .expect("server start");
    (port, graph, handle)
}

fn temp_port_file() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("memory.port");
    (dir, path)
}

// ═══════════════════════════════════════════════════════════════
// Protocol tests
// ═══════════════════════════════════════════════════════════════

#[test]
fn serialize_request() {
    let json = make_request(1, "ping", serde_json::json!({}));
    assert!(json.ends_with('\n'), "must be newline-terminated");
    let parsed: Request = serde_json::from_str(json.trim()).expect("valid JSON");
    assert_eq!(parsed.id, 1);
    assert_eq!(parsed.method, "ping");
}

#[test]
fn parse_response_ok() {
    let line = r#"{"id":1,"result":"pong","error":null}"#;
    let resp = parse_response(line).expect("parse ok");
    assert_eq!(resp.id, 1);
    assert_eq!(resp.result, Some(serde_json::json!("pong")));
    assert!(resp.error.is_none());
}

#[test]
fn parse_response_error() {
    let line = r#"{"id":2,"result":null,"error":{"message":"not found"}}"#;
    let resp = parse_response(line).expect("parse error response");
    assert_eq!(resp.id, 2);
    assert!(resp.result.is_none());
    assert_eq!(resp.error.as_ref().expect("has error").message, "not found");
}

#[test]
fn parse_response_malformed() {
    let result = parse_response("not json at all {{{");
    assert!(result.is_err());
}

// ═══════════════════════════════════════════════════════════════
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

    // 4. update_importance
    let resp = send_recv(
        &mut writer,
        &mut buf_reader,
        5,
        "update_importance",
        serde_json::json!({"id": id_a, "importance": 0.99}),
    )
    .await;
    assert!(
        resp.error.is_none(),
        "update_importance failed: {:?}",
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

// ═══════════════════════════════════════════════════════════════
// MemoryTrait tests
// ═══════════════════════════════════════════════════════════════

#[test]
fn direct_memory_trait() {
    let graph = MemoryGraph::in_memory().expect("graph");
    let mt: &dyn MemoryTrait = &graph;

    let id = mt
        .store_memory(
            "trait test",
            "tt",
            MemoryType::Fact,
            0.5,
            &["tag".to_string()],
            "test",
            "/tmp",
        )
        .expect("store");

    let mem = mt.get_memory(&id).expect("get");
    assert_eq!(mem.content, "trait test");

    mt.update_memory(&id, Some("updated"), None)
        .expect("update");
    let mem2 = mt.get_memory(&id).expect("get2");
    assert_eq!(mem2.content, "updated");

    mt.update_importance(&id, 0.99).expect("importance");

    let recalled = mt.recall_memories("updated", 10).expect("recall");
    assert!(!recalled.is_empty());

    let searched = mt
        .search_memories(&SearchFilter {
            memory_type: Some(MemoryType::Fact),
            ..Default::default()
        })
        .expect("search");
    assert!(!searched.is_empty());

    let recent = mt.list_recent(10).expect("recent");
    assert_eq!(recent.len(), 1);

    let count = mt.memory_count().expect("count");
    assert_eq!(count, 1);

    // Store second for relationship
    let id2 = mt
        .store_memory(
            "second",
            "s",
            MemoryType::Decision,
            0.5,
            &[],
            "test",
            "/tmp",
        )
        .expect("store2");

    mt.create_relationship(&id, &id2, RelType::RelatedTo, Some("test"), 0.8)
        .expect("rel");

    let related = mt.get_related_memories(&id, 1).expect("related");
    assert_eq!(related.len(), 1);

    mt.delete_memory(&id2).expect("delete");
    assert_eq!(mt.memory_count().expect("c"), 1);

    let cleared = mt.clear_all().expect("clear");
    assert_eq!(cleared, 1);
    assert_eq!(mt.memory_count().expect("c"), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remote_memory_trait() {
    let (_dir, port_file) = temp_port_file();
    let (port, _graph, handle) = start_test_server(port_file).await;

    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().expect("addr");
    let client = MemoryClient::connect(addr).await.expect("connect");

    // Use MemoryTrait through the client (via MemoryAccess::Remote)
    let access = MemoryAccess::Remote(client);
    let mt: &dyn MemoryTrait = &access;

    let id = mt
        .store_memory(
            "remote trait test",
            "rtt",
            MemoryType::Pattern,
            0.7,
            &["remote".to_string()],
            "test",
            "/tmp",
        )
        .expect("store");

    let mem = mt.get_memory(&id).expect("get");
    assert_eq!(mem.content, "remote trait test");
    assert_eq!(mem.memory_type, MemoryType::Pattern);

    mt.update_memory(&id, Some("updated remote"), None)
        .expect("update");
    let mem2 = mt.get_memory(&id).expect("get2");
    assert_eq!(mem2.content, "updated remote");

    mt.update_importance(&id, 0.95).expect("importance");

    let recalled = mt.recall_memories("remote", 10).expect("recall");
    assert!(!recalled.is_empty());

    let searched = mt
        .search_memories(&SearchFilter {
            memory_type: Some(MemoryType::Pattern),
            ..Default::default()
        })
        .expect("search");
    assert!(!searched.is_empty());

    let recent = mt.list_recent(10).expect("recent");
    assert_eq!(recent.len(), 1);

    assert_eq!(mt.memory_count().expect("count"), 1);

    let id2 = mt
        .store_memory("second", "s", MemoryType::Fact, 0.5, &[], "test", "/tmp")
        .expect("store2");

    mt.create_relationship(&id, &id2, RelType::CausedBy, None, 0.6)
        .expect("rel");

    let related = mt.get_related_memories(&id, 1).expect("related");
    assert_eq!(related.len(), 1);

    mt.delete_memory(&id2).expect("delete");
    assert_eq!(mt.memory_count().expect("c"), 1);

    let cleared = mt.clear_all().expect("clear");
    assert_eq!(cleared, 1);
    assert_eq!(mt.memory_count().expect("c"), 0);

    handle.abort();
}

// ═══════════════════════════════════════════════════════════════
// Access factory tests
// ═══════════════════════════════════════════════════════════════

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn open_memory_first_session_becomes_server() {
    let dir = tempfile::tempdir().expect("tempdir");
    let access = open_memory(dir.path()).await.expect("open");
    assert!(
        matches!(access, MemoryAccess::Direct { .. }),
        "first session should be Direct"
    );

    // Port file should exist
    let port_file = dir.path().join("memory.port");
    assert!(port_file.exists(), "port file should be created");

    // Should function as a memory store
    let mt: &dyn MemoryTrait = &access;
    let id = mt
        .store_memory(
            "factory test",
            "ft",
            MemoryType::Fact,
            0.5,
            &[],
            "t",
            "/tmp",
        )
        .expect("store");
    let mem = mt.get_memory(&id).expect("get");
    assert_eq!(mem.content, "factory test");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn open_memory_second_session_becomes_client() {
    let dir = tempfile::tempdir().expect("tempdir");

    // First session starts the server
    let _first = open_memory(dir.path()).await.expect("first open");
    assert!(matches!(_first, MemoryAccess::Direct { .. }));

    // Second session should connect as client
    let second = open_memory(dir.path()).await.expect("second open");
    assert!(
        matches!(second, MemoryAccess::Remote(_)),
        "second session should be Remote"
    );

    // Store via first, retrieve via second
    let mt1: &dyn MemoryTrait = &_first;
    let id = mt1
        .store_memory(
            "shared memory",
            "sm",
            MemoryType::Fact,
            0.8,
            &[],
            "t",
            "/tmp",
        )
        .expect("store via first");

    let mt2: &dyn MemoryTrait = &second;
    let mem = mt2.get_memory(&id).expect("get via second");
    assert_eq!(mem.content, "shared memory");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn open_memory_stale_port_cleaned() {
    let dir = tempfile::tempdir().expect("tempdir");
    let port_file = dir.path().join("memory.port");

    // Write a stale port file pointing to a port nothing listens on
    std::fs::write(&port_file, "65432").expect("write stale port");
    assert!(port_file.exists());

    // open_memory should detect the stale port, clean it up, and become server
    let access = open_memory(dir.path()).await.expect("open with stale port");
    assert!(
        matches!(access, MemoryAccess::Direct { .. }),
        "should become server after cleaning stale port"
    );

    // New port file should be written with the actual port
    let contents = std::fs::read_to_string(&port_file).expect("read port file");
    let port: u16 = contents.trim().parse().expect("valid port");
    assert_ne!(port, 65432, "should not still have stale port");
}
