//! `MockMemoryTrait` — test double for the memory layer
//! (REQ-FOR-PRESERVE-D8).
//!
//! Phase-8 will add a real `archon_memory::MemoryTrait`. Until that
//! trait is stable we keep a *local* trait `MemoryTraitLike` with the
//! same public surface (store / recall / search) and a recording
//! implementation. The regression guard in phase-8 can then assert
//! that any code path touching memory goes through the real trait
//! by swapping `MockMemoryTrait` in and inspecting the call log.
//!
//! This file MUST NOT `use archon_memory::*` — phase-0 does not
//! touch production crates. The shape is verified visually against
//! the real crate during phase-8; any drift is caught by the
//! acceptance test phase-8 writes.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// A single `store_memory` call recorded by [`MockMemoryTrait`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredMemory {
    pub content: String,
    pub tags: Vec<String>,
}

/// Minimal local mirror of the memory trait surface. Phase-8 may
/// either delete this in favour of a blanket impl of the real trait
/// or keep it as an ergonomic test-only alias.
#[async_trait]
pub trait MemoryTraitLike: Send + Sync {
    async fn store_memory(&self, content: &str, tags: &[String]) -> anyhow::Result<()>;
}

/// Recording double. Cheap to clone; the log is shared.
#[derive(Debug, Clone, Default)]
pub struct MockMemoryTrait {
    calls: Arc<Mutex<Vec<StoredMemory>>>,
}

impl MockMemoryTrait {
    pub fn new() -> Self {
        Self::default()
    }

    /// Snapshot of every recorded store call in insertion order.
    pub fn calls(&self) -> Vec<StoredMemory> {
        self.calls
            .lock()
            .expect("MockMemoryTrait call log poisoned")
            .clone()
    }
}

#[async_trait]
impl MemoryTraitLike for MockMemoryTrait {
    async fn store_memory(&self, content: &str, tags: &[String]) -> anyhow::Result<()> {
        self.calls
            .lock()
            .expect("MockMemoryTrait call log poisoned")
            .push(StoredMemory {
                content: content.to_string(),
                tags: tags.to_vec(),
            });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn calls_returns_recorded_stores_in_order() {
        let m = MockMemoryTrait::new();
        m.store_memory("alpha", &["a".into()]).await.unwrap();
        m.store_memory("beta", &["b".into(), "c".into()]).await.unwrap();
        let calls = m.calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].content, "alpha");
        assert_eq!(calls[1].tags, vec!["b".to_string(), "c".to_string()]);
    }
}
