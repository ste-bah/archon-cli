const DEFAULT_BATCH_SIZE: usize = 64;

#[derive(Clone, Debug)]
pub struct IndexOptions {
    pub all: bool,
    pub document_id: Option<String>,
    pub batch_size: usize,
    pub limit: Option<usize>,
    pub embedding_workers: Option<usize>,
    pub max_in_flight_batches: Option<usize>,
    pub writer_batch_size: Option<usize>,
}

impl Default for IndexOptions {
    fn default() -> Self {
        Self {
            all: false,
            document_id: None,
            batch_size: DEFAULT_BATCH_SIZE,
            limit: None,
            embedding_workers: None,
            max_in_flight_batches: None,
            writer_batch_size: None,
        }
    }
}

impl IndexOptions {
    pub fn effective_embedding_workers(&self, provider: &str) -> usize {
        self.embedding_workers
            .or_else(|| env_usize("ARCHON_DOCS_INDEX_EMBEDDING_WORKERS"))
            .unwrap_or_else(|| default_workers(provider))
            .max(1)
    }

    pub fn effective_max_in_flight_batches(&self, workers: usize) -> usize {
        self.max_in_flight_batches
            .or_else(|| env_usize("ARCHON_DOCS_INDEX_MAX_IN_FLIGHT_BATCHES"))
            .unwrap_or(workers.max(1))
            .max(1)
    }

    pub fn effective_writer_batch_size(&self) -> usize {
        self.writer_batch_size
            .or_else(|| env_usize("ARCHON_DOCS_INDEX_WRITER_BATCH_SIZE"))
            .unwrap_or(256)
            .max(1)
    }
}

fn default_workers(provider: &str) -> usize {
    if provider == "openai-compatible" {
        2
    } else {
        1
    }
}

fn env_usize(name: &str) -> Option<usize> {
    std::env::var(name).ok()?.parse().ok()
}
