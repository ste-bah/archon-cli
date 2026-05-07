// ---------------------------------------------------------------------------
// PipelineMemoryCoordinator
// ---------------------------------------------------------------------------

/// A pending memory store operation.
#[derive(Debug, Clone)]
pub struct PendingStore {
    pub key: String,
    pub value: String,
    pub priority: u32,
}

/// Coordinates memory operations across pipeline agents.
///
/// Queues store operations sorted by priority and provides batch flush.
pub struct PipelineMemoryCoordinator {
    pending: Vec<PendingStore>,
    total_flushes: usize,
}

impl PipelineMemoryCoordinator {
    pub fn new() -> Self {
        Self {
            pending: Vec::new(),
            total_flushes: 0,
        }
    }

    /// Queue a store operation, keeping the queue sorted by priority (highest first).
    pub fn coordinate_store(&mut self, key: &str, value: &str, priority: u32) {
        let entry = PendingStore {
            key: key.to_string(),
            value: value.to_string(),
            priority,
        };

        // Insert in sorted position (descending by priority).
        let pos = self
            .pending
            .iter()
            .position(|p| p.priority < priority)
            .unwrap_or(self.pending.len());
        self.pending.insert(pos, entry);
    }

    /// Look up a pending store by key.
    pub fn coordinate_recall(&self, key: &str) -> Option<&PendingStore> {
        self.pending.iter().find(|p| p.key == key)
    }

    /// Flush all pending stores, returning them in priority order.
    ///
    /// Clears the internal queue.
    pub fn flush(&mut self) -> Vec<PendingStore> {
        self.total_flushes += 1;
        std::mem::take(&mut self.pending)
    }

    /// Number of pending store operations.
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// Total number of flush operations performed.
    pub fn total_flushes(&self) -> usize {
        self.total_flushes
    }
}

impl Default for PipelineMemoryCoordinator {
    fn default() -> Self {
        Self::new()
    }
}
