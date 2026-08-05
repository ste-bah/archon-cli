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
    Ingest {
        /// Path to file or directory to ingest
        path: String,
        /// Skip the pre-ingest enrichment-classification confirmation prompt (batch/scripted use)
        #[arg(long, short = 'y')]
        yes: bool,
        /// Image-enrichment concurrency: "auto" (derive from free VRAM, confirm when
        /// interactive) or a number 1..=16. Unset -> the policy value (default 1 = serial).
        #[arg(long)]
        jobs: Option<String>,
    },
    /// Re-run OCR/VLM/image enrichment for an existing document ID or source path/prefix
    Reprocess {
        /// Document ID, source path, or source path prefix
        target: String,
        /// Do not run semantic indexing after reprocess; run `docs index` later
        #[arg(long)]
        defer_index: bool,
    },
    /// Permanently delete an existing document ID or source path/prefix and all its evidence
    Delete {
        /// Document ID, source path, or source path prefix
        target: String,
        /// Confirm deletion when the target matches more than one document
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// List all ingested documents
    List,
    /// Show detailed information about a document
    Show {
        /// Document ID
        document_id: String,
    },
    /// Show document status summary
    Status,
    /// List chunks for a document
    Chunks {
        /// Document ID
        document_id: String,
    },
    /// Full inspection of a document (pages, chunks, OCR runs, provenance)
    Inspect {
        /// Document ID
        document_id: String,
    },
    /// Search for chunks relevant to a query
    Search {
        /// Search query
        query: String,
        /// Retrieval mode: exact, semantic, or hybrid
        #[arg(long, default_value = "hybrid")]
        mode: String,
        /// Show debug output (embedding details, distances, provenance)
        #[arg(long)]
        debug: bool,
    },
    /// Search images/frames by a text description (cross-modal CLIP text→image)
    SearchImages {
        /// Text description to match against image embeddings
        query: String,
        /// Maximum results
        #[arg(long, default_value = "10")]
        limit: usize,
    },
    /// Answer a question using document evidence
    Answer {
        /// Question to answer
        query: String,
    },
    /// Show provenance chain for a chunk or answer component
    Provenance {
        /// Chunk ID or answer component ID
        chunk_or_answer_id: String,
    },
    /// Index document chunks (embed and store vectors)
    Index {
        /// Re-index all chunks regardless of status
        #[arg(long)]
        all: bool,
        /// Restrict indexing to one document ID
        #[arg(long, alias = "doc")]
        document: Option<String>,
        /// Number of chunks to embed per provider request
        #[arg(long, default_value_t = 64)]
        batch_size: usize,
        /// Maximum candidate chunks to process in this run
        #[arg(long)]
        limit: Option<usize>,
    },
    /// Show durable semantic-index queue counts
    IndexStatus,
    /// Requeue failed semantic-index chunks
    IndexRetryFailed {
        /// Maximum failed queue rows to retry
        #[arg(long)]
        limit: Option<usize>,
    },
    /// Pause an index job after its current window
    IndexPause {
        /// Index job ID
        job_id: String,
    },
    /// Resume a paused index job marker
    IndexResume {
        /// Index job ID
        job_id: String,
    },
    /// Cancel an index job and leave queue work retryable
    IndexCancel {
        /// Index job ID
        job_id: String,
    },
    /// Manage the background semantic-index worker
    IndexDaemon {
        #[command(subcommand)]
        action: DocsIndexDaemonAction,
    },
    /// Show Cozo/RocksDB/Rust-HNSW vector backend status
    VectorStatus,
    /// Migrate existing Cozo vectors into the RocksDB raw-vector store
    VectorMigrate {
        /// Maximum legacy vector rows to migrate in this run
        #[arg(long)]
        limit: Option<usize>,
        /// RocksDB write batch size
        #[arg(long, default_value_t = 1024)]
        batch_size: usize,
        /// Resume after this chunk id
        #[arg(long)]
        after: Option<String>,
    },
    /// Build a Rust-HNSW snapshot from RocksDB raw vectors
    VectorCompact {
        /// Provider/backend name to compact
        #[arg(long)]
        provider: Option<String>,
        /// Embedding dimension; defaults to the active provider dimension
        #[arg(long)]
        dimension: Option<usize>,
        /// Maximum raw vectors to include
        #[arg(long)]
        limit: Option<usize>,
    },
    /// Report embedding model and backend status
    ModelStatus,
    /// Verify a quote against the corpus — locate its source document, page(s), and bbox(es)
    VerifyQuote {
        /// The quote text to locate (verbatim; smart quotes + whitespace are normalized)
        quote: String,
        /// Restrict the search to a single document ID
        #[arg(long)]
        doc: Option<String>,
        /// Maximum number of source locations to report
        #[arg(long, default_value = "3")]
        limit: usize,
        /// Emit machine-readable JSON
        #[arg(long)]
        json: bool,
    },
    /// Verify chunk-integrity (chunks_root) for one document or all documents
    VerifyIntegrity {
        /// Restrict verification to a single document ID (default: all documents)
        #[arg(long)]
        doc: Option<String>,
        /// Emit machine-readable JSON
        #[arg(long)]
        json: bool,
    },
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

/// Actions for `archon corpus-index` — the claim/clause-level corpus index (C1 port).
#[derive(clap::Subcommand, Debug, Clone, PartialEq)]
pub enum CorpusIndexAction {
    /// Create the corpus_* relations (idempotent)
    EnsureSchema,
    /// Row counts per relation + import audit trail
    Status,
    /// Dry-run validate a JSONL/JSON-array intermediate against the relation contract
    Validate {
        /// Record kind: sources|clauses|claims|edges|tensions|terms|groups
        kind: String,
        /// Path to the intermediate file
        file: std::path::PathBuf,
    },
    /// E0 ingestion-measurement pass: per-document bbox/spatial/locator coverage + probes
    Probes,
    /// Dump all records of a kind as JSONL (optionally filtered by entry_id)
    Dump {
        /// Record kind: sources|clauses|claims|edges|tensions|terms|groups
        kind: String,
        /// Restrict to one entry_id (kinds that carry it)
        #[arg(long)]
        entry: Option<String>,
    },
    /// Show one record by id (JSON)
    Show {
        /// Record kind: sources|clauses|claims|edges|tensions|terms|groups
        kind: String,
        /// The record's key value
        id: String,
    },
    /// Validate + batch-import an intermediate; rejected records are quarantined, never dropped
    Import {
        /// Record kind: sources|clauses|claims|edges|tensions|terms|groups
        kind: String,
        /// Path to the intermediate file
        file: std::path::PathBuf,
        /// Quarantine sidecar path (default: <file>.quarantine.jsonl)
        #[arg(long)]
        quarantine: Option<std::path::PathBuf>,
        /// Validate and report only; write nothing
        #[arg(long)]
        dry_run: bool,
        /// Clauses only: verify each anchored row's quote against its pinned document
        /// (verify-quote; exact or similarity >= 0.90). Rows that fail are quarantined,
        /// never written — entries are born verified. Unanchored rows (no archon
        /// text_layer_id) skip the gate. Combines with --dry-run for a no-write audit.
        #[arg(long)]
        verify_quotes: bool,
    },
    /// Remove records by id (verifies existence; requires --yes)
    Remove {
        /// Record kind: sources|clauses|claims|edges|tensions|terms|groups
        kind: String,
        /// The record's key value
        id: Option<String>,
        /// File with one key value per line (batch removal)
        #[arg(long)]
        ids_file: Option<std::path::PathBuf>,
        /// Actually remove (without this flag: report only and exit non-zero)
        #[arg(short = 'y', long)]
        yes: bool,
    },
}
