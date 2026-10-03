use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum RemoteAction {
    /// Connect to a remote agent via SSH
    Ssh {
        /// Target in user@host format (defaults to root@host if no @ present)
        target: String,
        /// One-shot command to run on the remote agent
        #[arg(long)]
        command: Option<String>,
        /// SSH port
        #[arg(long, default_value = "22")]
        port: u16,
        /// Path to SSH private key file
        #[arg(long)]
        key: Option<std::path::PathBuf>,
    },
    /// Connect to a remote agent via WebSocket
    Ws {
        /// WebSocket URL (e.g. ws://host:8420/ws)
        url: String,
        /// Bearer token for authentication
        #[arg(long)]
        token: Option<String>,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum KbAction {
    /// Ingest a file, URL, or directory into the knowledge base
    Ingest {
        /// Path or URL to ingest
        source: String,
        /// Knowledge-base name to attach the ingested source to
        #[arg(long, alias = "domain")]
        kb: Option<String>,
    },
    /// List all nodes in the knowledge base
    List {
        /// Restrict output to a named knowledge base
        #[arg(long)]
        kb: Option<String>,
    },
    /// Search for nodes matching a query string
    Search {
        /// Search query
        query: String,
        /// Maximum results
        #[arg(long, default_value = "10")]
        limit: usize,
        /// Retrieval mode: exact, semantic, or hybrid
        #[arg(long, default_value = "hybrid")]
        mode: String,
        /// Restrict results to a named knowledge base
        #[arg(long)]
        kb: Option<String>,
    },
    /// Recall across memory, docs, the knowledge graph and the code index at once
    ///
    /// Scores are UNCALIBRATED: they encode each store's own ranking and carry
    /// no cross-store meaning. See `archon_knowledge::recall::normalize`.
    Recall {
        /// Search query
        query: String,
        /// Maximum merged results, split evenly as a per-source quota
        #[arg(long, default_value = "10")]
        limit: usize,
        /// Comma-separated stores to consult: memory, docs, knowledge, code
        #[arg(long, default_value = "memory,docs,knowledge,code")]
        sources: String,
        /// How long any one store may take before it is abandoned
        #[arg(long, default_value = "5000")]
        source_timeout_ms: u64,
        /// Path to an already-built LEANN code index; without it the code
        /// source is reported as not consulted rather than skipped silently
        #[arg(long)]
        code_index: Option<std::path::PathBuf>,
        /// Retrieval mode for the knowledge graph source: exact, semantic, hybrid
        #[arg(long, default_value = "hybrid")]
        mode: String,
        /// Restrict the knowledge graph source to a named knowledge base
        #[arg(long)]
        kb: Option<String>,
    },
    /// Extract claims, entities, relations, source quality and contradictions from doc chunks
    Process {
        /// Extract claims from document chunks
        #[arg(long)]
        claims: bool,
        /// Extract entities from document chunks
        #[arg(long)]
        entities: bool,
        /// Infer the knowledge graph relations
        #[arg(long, alias = "kg")]
        relations: bool,
        /// Scan claims for contradictions
        #[arg(long)]
        contradictions: bool,
        /// Restrict processing to a named knowledge base
        #[arg(long)]
        kb: Option<String>,
    },
    /// Re-run OCR/VLM/image enrichment for every document in a knowledge base
    Reprocess {
        /// Knowledge-base name to reprocess
        #[arg(long, alias = "domain")]
        kb: String,
        /// Do not run semantic indexing after reprocess; run `docs index` later
        #[arg(long)]
        defer_index: bool,
    },
    /// List every knowledge base in the store with its document count
    ///
    /// Every other verb here takes `--kb <name>` as an input filter, which
    /// assumes you already know the name. This is the verb that tells you.
    Kbs,
    /// List extracted claims
    Claims,
    /// List extracted entities
    Entities,
    /// List inferred relations
    Relations,
    /// List detected contradictions
    Contradictions,
    /// Show knowledge base statistics
    Stats,
}

#[derive(Subcommand, Debug, Clone)]
pub enum DocsAction {
    /// Ingest a file or directory
    Ingest(DocsIngestArgs),
    /// Re-run OCR/VLM/image enrichment for an existing document ID or source path/prefix
    Reprocess(DocsReprocessArgs),
    /// Permanently delete an existing document ID or source path/prefix and all its evidence
    Delete(DocsDeleteArgs),
    /// List all ingested documents
    List,
    /// Show detailed information about a document
    Show(DocsShowArgs),
    /// Show document status summary
    Status,
    /// List chunks for a document
    Chunks(DocsChunksArgs),
    /// Full inspection of a document (pages, chunks, OCR runs, provenance)
    Inspect(DocsInspectArgs),
    /// Search for chunks relevant to a query
    Search(DocsSearchArgs),
    /// Search images/frames by a text description (cross-modal CLIP text→image)
    SearchImages(DocsSearchImagesArgs),
    /// Compile ingested documents into summaries, concept articles and an index
    ///
    /// REQ-KB-002. Reads `doc_chunks` and writes its output back as ordinary
    /// documents, so `docs search`, `kb search` and `kb recall` see the results
    /// immediately. NFR-PIPE-012 budgets 5 minutes for 20 documents.
    Compile(DocsCompileArgs),
    /// Export the corpus to markdown, grouped into raw/compiled/concepts/answers/index
    Export(DocsExportArgs),
    /// Answer a question using document evidence
    ///
    /// REQ-DOCS-013/014/015, the same capability REQ-KB-003 specifies. Uses LLM
    /// synthesis when a provider is configured and the extractive path when one
    /// is not; either way, insufficient evidence is reported rather than
    /// papered over.
    Answer(DocsAnswerArgs),
    /// Show provenance chain for a chunk or answer component
    Provenance(DocsProvenanceArgs),
    /// Index document chunks (embed and store vectors)
    Index(DocsIndexArgs),
    /// Show durable semantic-index queue counts
    IndexStatus,
    /// Requeue failed semantic-index chunks
    IndexRetryFailed(DocsIndexRetryFailedArgs),
    /// Pause an index job after its current window
    IndexPause(DocsIndexPauseArgs),
    /// Resume a paused index job marker
    IndexResume(DocsIndexResumeArgs),
    /// Cancel an index job and leave queue work retryable
    IndexCancel(DocsIndexCancelArgs),
    /// Manage the background semantic-index worker
    IndexDaemon {
        #[command(subcommand)]
        action: DocsIndexDaemonAction,
    },
    /// Show Cozo/RocksDB/Rust-HNSW vector backend status
    VectorStatus,
    /// Migrate existing Cozo vectors into the RocksDB raw-vector store
    VectorMigrate(DocsVectorMigrateArgs),
    /// Build a Rust-HNSW snapshot from RocksDB raw vectors
    VectorCompact(DocsVectorCompactArgs),
    /// Report embedding model and backend status
    ModelStatus,
    /// Verify a quote against the corpus — locate its source document, page(s), and bbox(es)
    VerifyQuote(DocsVerifyQuoteArgs),
    /// Verify chunk-integrity (chunks_root) for one document or all documents
    VerifyIntegrity(DocsVerifyIntegrityArgs),
}

#[derive(Subcommand, Debug, Clone)]
pub enum DocsIndexDaemonAction {
    /// Start a background docs index worker for the current project
    Start {
        /// Number of chunks to embed per provider request
        #[arg(long, default_value_t = 64)]
        batch_size: usize,
        /// Maximum queued chunks to drain per daemon pass
        #[arg(long, default_value_t = 1024)]
        window_size: usize,
        /// Seconds to wait between empty queue polls
        #[arg(long, default_value_t = 30)]
        poll_secs: u64,
    },
    /// Stop the background docs index worker for the current project
    Stop,
    /// Show daemon pid/log status for the current project
    Status,
    /// Internal foreground loop used by `start`
    #[command(hide = true)]
    Run {
        /// Number of chunks to embed per provider request
        #[arg(long, default_value_t = 64)]
        batch_size: usize,
        /// Maximum queued chunks to drain per daemon pass
        #[arg(long, default_value_t = 1024)]
        window_size: usize,
        /// Seconds to wait between empty queue polls
        #[arg(long, default_value_t = 30)]
        poll_secs: u64,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum ProvAction {
    /// Trace an artifact to its source lineage
    Trace {
        /// Artifact ID to trace
        artifact_id: String,
    },
    /// Export an artifact trace as W3C PROV JSON-LD
    Export {
        /// Artifact ID to export
        artifact_id: String,
    },
    /// Verify an artifact trace reaches source provenance
    Verify {
        /// Artifact ID to verify
        artifact_id: String,
    },
}

#[path = "data_actions_agent.rs"]
mod agent;
pub use agent::*;

#[path = "data_actions_args.rs"]
pub(super) mod data_actions_args;
pub use data_actions_args::*;
