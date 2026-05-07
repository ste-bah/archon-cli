use std::sync::Arc;

use crate::graph::MemoryGraph;
use crate::types::MemoryType;

use super::{MemoryAccess, MemoryTrait};

/// Verify that `Arc<dyn MemoryTrait>` works with a concrete `MemoryGraph`,
/// proving the trait is object-safe and usable polymorphically.
#[test]
fn arc_dyn_memory_trait_store_and_recall() {
    let graph = MemoryGraph::in_memory().expect("in-memory graph");
    let mem: Arc<dyn MemoryTrait> = Arc::new(graph);

    let id = mem
        .store_memory(
            "Rust async uses tokio runtime",
            "async note",
            MemoryType::Fact,
            0.8,
            &["rust".into(), "async".into()],
            "test",
            "/test",
        )
        .expect("store_memory via trait object");

    let recalled = mem
        .recall_memories("tokio", 5)
        .expect("recall via trait object");
    assert!(!recalled.is_empty(), "should recall at least one memory");
    assert_eq!(recalled[0].id, id);
}

/// Verify that `MemoryAccess` (the enum) works as `Arc<dyn MemoryTrait>`.
#[test]
fn memory_access_as_dyn_trait() {
    let graph = MemoryGraph::in_memory().expect("in-memory graph");
    let access = MemoryAccess::Direct {
        graph: Arc::new(graph),
        _server_handle: tokio::runtime::Runtime::new().unwrap().spawn(async {}),
    };
    let mem: Arc<dyn MemoryTrait> = Arc::new(access);

    let _id = mem
        .store_memory(
            "test content",
            "title",
            MemoryType::Fact,
            0.5,
            &[],
            "test",
            "",
        )
        .expect("store via MemoryAccess trait object");

    assert_eq!(mem.memory_count().expect("count"), 1);
}
