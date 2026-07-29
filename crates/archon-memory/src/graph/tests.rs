use std::sync::{Arc, Barrier, mpsc};
use std::thread;

use super::*;
use crate::types::{MemoryError, MemoryType, RelType, SearchFilter};

fn make_graph() -> MemoryGraph {
    MemoryGraph::in_memory().expect("failed to create in-memory graph")
}

#[test]
fn store_and_get() {
    let g = make_graph();
    let id = g
        .store_memory(
            "Rust is great",
            "rust fact",
            MemoryType::Fact,
            0.8,
            &["rust".into(), "lang".into()],
            "manual",
            "/tmp",
        )
        .expect("store failed");

    let m = g.get_memory(&id).expect("get failed");
    assert_eq!(m.content, "Rust is great");
    assert_eq!(m.memory_type, MemoryType::Fact);
    assert_eq!(m.tags, vec!["rust", "lang"]);
    assert_eq!(m.access_count, 1);
}

#[cfg(test)]
mod concurrency;

#[test]
fn explicit_id_rejects_authoritative_identity_collision() {
    let graph = make_graph();
    graph
        .store_memory_with_id(
            "rule:correction:collision",
            "Avoid: first content",
            "",
            MemoryType::Rule,
            50.0,
            &["source:correction-derived".into(), "trend:stable".into()],
            "rules_engine",
            "",
        )
        .expect("create rule");

    let error = graph
        .store_memory_with_id(
            "rule:correction:collision",
            "Avoid: mismatched content",
            "",
            MemoryType::Rule,
            50.0,
            &["source:correction-derived".into(), "trend:stable".into()],
            "rules_engine",
            "",
        )
        .expect_err("collision must fail");
    assert!(matches!(error, MemoryError::Database(message) if message.contains("collision")));
}

#[test]
fn get_missing_returns_not_found() {
    let g = make_graph();
    let err = g.get_memory("nonexistent").unwrap_err();
    assert!(matches!(err, MemoryError::NotFound(_)));
}

#[test]
fn get_memory_updates_access_stats_from_the_current_database_row() {
    const READERS: usize = 32;

    let graph = Arc::new(make_graph());
    let id = graph
        .store_memory(
            "rule",
            "",
            MemoryType::Rule,
            50.0,
            &["tag".into()],
            "test",
            "",
        )
        .expect("store rule");
    let barrier = Arc::new(Barrier::new(READERS + 1));
    let (tx, rx) = mpsc::channel();

    for _ in 0..READERS {
        let graph = Arc::clone(&graph);
        let barrier = Arc::clone(&barrier);
        let id = id.clone();
        let tx = tx.clone();
        thread::spawn(move || {
            barrier.wait();
            tx.send(graph.get_memory(&id)).expect("send access result");
        });
    }

    barrier.wait();
    for _ in 0..READERS {
        let accessed = rx
            .recv()
            .expect("receive access result")
            .expect("access memory");
        assert_eq!(accessed.content, "rule");
        assert_eq!(accessed.tags, ["tag"]);
        assert!(accessed.last_accessed.is_some());
    }

    let stored = graph.read_memory(&id).expect("read source of truth");
    assert_eq!(stored.access_count, READERS as u64);
    assert!(stored.last_accessed.is_some());
}

#[cfg(test)]
mod delta;
#[cfg(test)]
mod delta_validation;

#[test]
fn update_memory_content_and_tags() {
    let g = make_graph();
    let id = g
        .store_memory("old", "", MemoryType::Decision, 0.5, &[], "m", "")
        .expect("store failed");
    g.update_memory(&id, Some("new"), Some(&["tag1".into()]))
        .expect("update failed");
    let m = g.get_memory(&id).expect("get failed");
    assert_eq!(m.content, "new");
    assert_eq!(m.tags, vec!["tag1"]);
    assert!(m.updated_at.is_some());
}

#[test]
fn update_nonexistent_errors() {
    let g = make_graph();
    let err = g.update_memory("nope", Some("x"), None).unwrap_err();
    assert!(matches!(err, MemoryError::NotFound(_)));
}

#[test]
fn delete_memory_and_relationships() {
    let g = make_graph();
    let a = g
        .store_memory("a", "", MemoryType::Fact, 0.5, &[], "m", "")
        .expect("store failed");
    let b = g
        .store_memory("b", "", MemoryType::Fact, 0.5, &[], "m", "")
        .expect("store failed");
    g.create_relationship(&a, &b, RelType::RelatedTo, None, 1.0)
        .expect("rel failed");
    g.delete_memory(&a).expect("delete failed");

    assert!(g.get_memory(&b).is_ok());
    assert!(g.get_memory(&a).is_err());
    let related = g.get_related_memories(&b, 1).expect("traversal failed");
    assert!(related.is_empty());
}

#[test]
fn deleting_recreated_deterministic_memory_clears_relationships_and_provenance_ledger() {
    let graph = make_graph();
    let id = "rule:correction:recreated";
    let provenance = "correction:recreated";
    let outgoing = graph
        .store_memory("outgoing", "", MemoryType::Fact, 50.0, &[], "test", "")
        .expect("create outgoing memory");
    let incoming = graph
        .store_memory("incoming", "", MemoryType::Fact, 50.0, &[], "test", "")
        .expect("create incoming memory");

    graph
        .store_memory_with_id(
            id,
            "Avoid: recreate safely",
            "",
            MemoryType::Rule,
            50.0,
            &[],
            "test",
            "",
        )
        .expect("create deterministic memory");
    graph
        .create_relationship(id, &outgoing, RelType::RelatedTo, None, 1.0)
        .expect("create outgoing relationship");
    graph
        .create_relationship(&incoming, id, RelType::RelatedTo, None, 1.0)
        .expect("create incoming relationship");
    graph
        .apply_importance_delta(id, 10.0, provenance)
        .expect("apply initial delta");
    graph.delete_memory(id).expect("delete memory");

    assert!(
        graph
            .get_related_memories(&outgoing, 1)
            .expect("read outgoing relationships")
            .is_empty()
    );
    assert!(
        graph
            .get_related_memories(&incoming, 1)
            .expect("read incoming relationships")
            .is_empty()
    );

    graph
        .store_memory_with_id(
            id,
            "Avoid: recreate safely",
            "",
            MemoryType::Rule,
            50.0,
            &[],
            "test",
            "",
        )
        .expect("recreate deterministic memory");
    let recreated = graph
        .apply_importance_delta(id, 10.0, provenance)
        .expect("apply provenance to recreated memory");

    assert_eq!(recreated.importance, 60.0);
    assert!(
        graph
            .has_importance_application(id, provenance)
            .expect("read recreated provenance")
    );
}

#[test]
fn delete_nonexistent_errors() {
    let g = make_graph();
    let err = g.delete_memory("nope").unwrap_err();
    assert!(matches!(err, MemoryError::NotFound(_)));
}

#[test]
fn graph_traversal_depth() {
    let g = make_graph();
    let a = g
        .store_memory("a", "", MemoryType::Fact, 0.5, &[], "m", "")
        .expect("store failed");
    let b = g
        .store_memory("b", "", MemoryType::Fact, 0.5, &[], "m", "")
        .expect("store failed");
    let c = g
        .store_memory("c", "", MemoryType::Fact, 0.5, &[], "m", "")
        .expect("store failed");
    let d = g
        .store_memory("d", "", MemoryType::Fact, 0.5, &[], "m", "")
        .expect("store failed");

    g.create_relationship(&a, &b, RelType::RelatedTo, None, 1.0)
        .expect("rel failed");
    g.create_relationship(&b, &c, RelType::CausedBy, None, 1.0)
        .expect("rel failed");
    g.create_relationship(&c, &d, RelType::DerivedFrom, None, 1.0)
        .expect("rel failed");

    let r1 = g.get_related_memories(&a, 1).expect("traversal failed");
    assert_eq!(r1.len(), 1);
    assert_eq!(r1[0].id, b);

    let r2 = g.get_related_memories(&a, 2).expect("traversal failed");
    assert_eq!(r2.len(), 2);

    let r3 = g.get_related_memories(&a, 3).expect("traversal failed");
    assert_eq!(r3.len(), 3);

    let r0 = g.get_related_memories(&a, 0).expect("traversal failed");
    assert!(r0.is_empty());
}

#[test]
fn recall_basic() {
    let g = make_graph();
    g.store_memory(
        "Rust memory safety is important",
        "safety",
        MemoryType::Fact,
        0.9,
        &["rust".into()],
        "manual",
        "",
    )
    .expect("store failed");
    g.store_memory(
        "Python is dynamically typed",
        "python",
        MemoryType::Fact,
        0.5,
        &["python".into()],
        "manual",
        "",
    )
    .expect("store failed");

    let results = g.recall_memories("rust safety", 10).expect("recall failed");
    assert!(!results.is_empty());
    assert!(results[0].content.contains("Rust"));
}

#[test]
fn recall_empty_query_returns_empty() {
    let g = make_graph();
    g.store_memory("x", "", MemoryType::Fact, 0.5, &[], "m", "")
        .expect("store failed");
    let r = g.recall_memories("", 10).expect("recall failed");
    assert!(r.is_empty());
}

#[test]
fn search_by_type() {
    let g = make_graph();
    g.store_memory("a", "", MemoryType::Fact, 0.5, &[], "m", "")
        .expect("store failed");
    g.store_memory("b", "", MemoryType::Decision, 0.5, &[], "m", "")
        .expect("store failed");

    let filter = SearchFilter {
        memory_type: Some(MemoryType::Decision),
        ..Default::default()
    };
    let results = g.search_memories(&filter).expect("search failed");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].content, "b");
}

#[test]
fn search_by_tags_any() {
    let g = make_graph();
    g.store_memory("a", "", MemoryType::Fact, 0.5, &["x".into()], "m", "")
        .expect("store failed");
    g.store_memory("b", "", MemoryType::Fact, 0.5, &["y".into()], "m", "")
        .expect("store failed");
    g.store_memory(
        "c",
        "",
        MemoryType::Fact,
        0.5,
        &["x".into(), "y".into()],
        "m",
        "",
    )
    .expect("store failed");

    let filter = SearchFilter {
        tags: vec!["x".into()],
        require_all_tags: false,
        ..Default::default()
    };
    let results = g.search_memories(&filter).expect("search failed");
    assert_eq!(results.len(), 2);
}

#[test]
fn search_by_tags_all() {
    let g = make_graph();
    g.store_memory("a", "", MemoryType::Fact, 0.5, &["x".into()], "m", "")
        .expect("store failed");
    g.store_memory(
        "c",
        "",
        MemoryType::Fact,
        0.5,
        &["x".into(), "y".into()],
        "m",
        "",
    )
    .expect("store failed");

    let filter = SearchFilter {
        tags: vec!["x".into(), "y".into()],
        require_all_tags: true,
        ..Default::default()
    };
    let results = g.search_memories(&filter).expect("search failed");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].content, "c");
}

#[test]
fn search_by_text() {
    let g = make_graph();
    g.store_memory("alpha beta", "", MemoryType::Fact, 0.5, &[], "m", "")
        .expect("store failed");
    g.store_memory("gamma delta", "", MemoryType::Fact, 0.5, &[], "m", "")
        .expect("store failed");

    let filter = SearchFilter {
        text: Some("beta".into()),
        ..Default::default()
    };
    let results = g.search_memories(&filter).expect("search failed");
    assert_eq!(results.len(), 1);
    assert!(results[0].content.contains("beta"));
}

#[test]
fn empty_graph_operations() {
    let g = make_graph();
    let r = g.recall_memories("anything", 10).expect("recall failed");
    assert!(r.is_empty());
    let s = g
        .search_memories(&SearchFilter::default())
        .expect("search failed");
    assert!(s.is_empty());
    let rel = g
        .get_related_memories("nonexistent", 3)
        .expect("traversal failed");
    assert!(rel.is_empty());
}

#[test]
fn persistence_across_reopen() {
    let dir = tempfile::tempdir().expect("tempdir failed");
    let db_path = dir.path().join("test.db");

    let id = {
        let g = MemoryGraph::open(&db_path).expect("open failed");
        g.store_memory("persist me", "", MemoryType::Rule, 0.9, &[], "m", "")
            .expect("store failed")
    };

    let g = MemoryGraph::open(&db_path).expect("reopen failed");
    let m = g.get_memory(&id).expect("get failed");
    assert_eq!(m.content, "persist me");
}
