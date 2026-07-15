use super::support::*;
use super::*;
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

    let atomically_updated = mt
        .apply_importance_delta(&id, 0.25, "direct-memory-trait")
        .expect("atomic delta");
    assert_eq!(atomically_updated.importance, 0.75);
    assert_eq!(atomically_updated.tags, vec!["tag", "trend:rising"]);

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
async fn remote_explicit_id_outcome_forwards_created_flag() {
    let (_dir, port_file) = temp_port_file();
    let (port, _graph, handle) = start_test_server(port_file).await;
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().expect("addr");
    let client = MemoryClient::connect(addr).await.expect("connect");
    let access = MemoryAccess::Remote(client);

    let created = access
        .store_memory_with_id_outcome(
            "remote:explicit-id-outcome",
            "remote explicit outcome",
            "",
            MemoryType::Rule,
            50.0,
            &["source:test".into(), "trend:stable".into()],
            "test",
            "",
        )
        .expect("create remote memory");
    let existing = access
        .store_memory_with_id_outcome(
            "remote:explicit-id-outcome",
            "remote explicit outcome",
            "",
            MemoryType::Rule,
            50.0,
            &["source:test".into(), "trend:stable".into()],
            "test",
            "",
        )
        .expect("read remote memory");

    assert!(created.created);
    assert!(!existing.created);
    assert_eq!(created.memory.id, existing.memory.id);
    handle.abort();
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

    let atomically_updated = mt
        .apply_importance_delta(&id, 0.15, "remote-memory-trait")
        .expect("atomic delta");
    assert_eq!(atomically_updated.importance, 0.85);
    assert_eq!(atomically_updated.tags, vec!["remote", "trend:rising"]);

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
