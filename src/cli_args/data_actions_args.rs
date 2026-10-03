//! Argument structs for `DocsAction` variants. Each struct gets its own
//! clap `augment_args` frame, so the frames do not stack (#233).

use clap::Args;

#[derive(Args, Debug, Clone)]
pub struct DocsIngestArgs {
    /// Path to file or directory to ingest
    pub path: String,
    /// Skip the pre-ingest enrichment-classification confirmation prompt (batch/scripted use)
    #[arg(long, short = 'y')]
    pub yes: bool,
    /// Image-enrichment concurrency: "auto" (derive from free VRAM, confirm when
    /// interactive) or a number 1..=16. Unset -> the policy value (default 1 = serial).
    #[arg(long)]
    pub jobs: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct DocsReprocessArgs {
    /// Document ID, source path, or source path prefix
    pub target: String,
    /// Do not run semantic indexing after reprocess; run `docs index` later
    #[arg(long)]
    pub defer_index: bool,
}

#[derive(Args, Debug, Clone)]
pub struct DocsDeleteArgs {
    /// Document ID, source path, or source path prefix
    pub target: String,
    /// Confirm deletion when the target matches more than one document
    #[arg(long, short = 'y')]
    pub yes: bool,
}

#[derive(Args, Debug, Clone)]
pub struct DocsShowArgs {
    /// Document ID
    pub document_id: String,
}

#[derive(Args, Debug, Clone)]
pub struct DocsChunksArgs {
    /// Document ID
    pub document_id: String,
}

#[derive(Args, Debug, Clone)]
pub struct DocsInspectArgs {
    /// Document ID
    pub document_id: String,
}

#[derive(Args, Debug, Clone)]
pub struct DocsSearchArgs {
    /// Search query
    pub query: String,
    /// Retrieval mode: exact, semantic, or hybrid
    #[arg(long, default_value = "hybrid")]
    pub mode: String,
    /// Show debug output (embedding details, distances, provenance)
    #[arg(long)]
    pub debug: bool,
}

#[derive(Args, Debug, Clone)]
pub struct DocsSearchImagesArgs {
    /// Text description to match against image embeddings
    pub query: String,
    /// Maximum results
    #[arg(long, default_value = "10")]
    pub limit: usize,
}

#[derive(Args, Debug, Clone)]
pub struct DocsCompileArgs {
    /// Restrict compilation to a named knowledge base
    #[arg(long, alias = "domain")]
    pub kb: Option<String>,
    /// Model alias or ID to compile with
    #[arg(long)]
    pub model: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct DocsExportArgs {
    /// Directory to write one markdown file per document; omit to print to stdout
    #[arg(long)]
    pub out: Option<std::path::PathBuf>,
    /// Restrict the export to a named knowledge base
    #[arg(long, alias = "domain")]
    pub kb: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct DocsAnswerArgs {
    /// Question to answer
    pub query: String,
    /// Use the extractive answer even when a provider is available
    #[arg(long)]
    pub no_synthesis: bool,
    /// File the answer back into the corpus as a searchable document
    #[arg(long)]
    pub file: bool,
    /// Restrict retrieval to a named knowledge base
    #[arg(long, alias = "domain")]
    pub kb: Option<String>,
    /// Maximum evidence chunks to retrieve
    #[arg(long, default_value = "5")]
    pub limit: usize,
    /// Retrieval mode: exact, semantic, or hybrid
    #[arg(long, default_value = "hybrid")]
    pub mode: String,
    /// Model alias or ID to synthesize with
    #[arg(long)]
    pub model: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct DocsProvenanceArgs {
    /// Chunk ID or answer component ID
    pub chunk_or_answer_id: String,
}

#[derive(Args, Debug, Clone)]
pub struct DocsIndexArgs {
    /// Re-index all chunks regardless of status
    #[arg(long)]
    pub all: bool,
    /// Restrict indexing to one document ID
    #[arg(long, alias = "doc")]
    pub document: Option<String>,
    /// Number of chunks to embed per provider request
    #[arg(long, default_value_t = 64)]
    pub batch_size: usize,
    /// Maximum candidate chunks to process in this run
    #[arg(long)]
    pub limit: Option<usize>,
}

#[derive(Args, Debug, Clone)]
pub struct DocsIndexRetryFailedArgs {
    /// Maximum failed queue rows to retry
    #[arg(long)]
    pub limit: Option<usize>,
}

#[derive(Args, Debug, Clone)]
pub struct DocsIndexPauseArgs {
    /// Index job ID
    pub job_id: String,
}

#[derive(Args, Debug, Clone)]
pub struct DocsIndexResumeArgs {
    /// Index job ID
    pub job_id: String,
}

#[derive(Args, Debug, Clone)]
pub struct DocsIndexCancelArgs {
    /// Index job ID
    pub job_id: String,
}

#[derive(Args, Debug, Clone)]
pub struct DocsVectorMigrateArgs {
    /// Maximum legacy vector rows to migrate in this run
    #[arg(long)]
    pub limit: Option<usize>,
    /// RocksDB write batch size
    #[arg(long, default_value_t = 1024)]
    pub batch_size: usize,
    /// Resume after this chunk id
    #[arg(long)]
    pub after: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct DocsVectorCompactArgs {
    /// Provider/backend name to compact
    #[arg(long)]
    pub provider: Option<String>,
    /// Embedding dimension; defaults to the active provider dimension
    #[arg(long)]
    pub dimension: Option<usize>,
    /// Maximum raw vectors to include
    #[arg(long)]
    pub limit: Option<usize>,
}

#[derive(Args, Debug, Clone)]
pub struct DocsVerifyQuoteArgs {
    /// The quote text to locate (verbatim; smart quotes + whitespace are normalized)
    pub quote: String,
    /// Restrict the search to a single document ID
    #[arg(long)]
    pub doc: Option<String>,
    /// Maximum number of source locations to report
    #[arg(long, default_value = "3")]
    pub limit: usize,
    /// Emit machine-readable JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug, Clone)]
pub struct DocsVerifyIntegrityArgs {
    /// Restrict verification to a single document ID (default: all documents)
    #[arg(long)]
    pub doc: Option<String>,
    /// Emit machine-readable JSON
    #[arg(long)]
    pub json: bool,
}
