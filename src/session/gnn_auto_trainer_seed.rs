use archon_memory::{MemoryTrait, MemoryType, SearchFilter};

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct GnnAutoTrainerSeed {
    pub total_memories: u64,
    pub total_corrections: u64,
}

pub(super) fn from_memory_graph(memory: &dyn MemoryTrait) -> GnnAutoTrainerSeed {
    let total_memories = match memory.memory_count() {
        Ok(count) => count as u64,
        Err(e) => {
            tracing::warn!(error = %e, "GNN auto-trainer memory seed count failed");
            0
        }
    };
    let filter = SearchFilter {
        memory_type: Some(MemoryType::Correction),
        ..Default::default()
    };
    let total_corrections = match memory.search_memories(&filter) {
        Ok(corrections) => corrections.len() as u64,
        Err(e) => {
            tracing::warn!(error = %e, "GNN auto-trainer correction seed count failed");
            0
        }
    };

    GnnAutoTrainerSeed {
        total_memories,
        total_corrections,
    }
}
