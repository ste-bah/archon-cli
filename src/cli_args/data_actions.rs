use std::path::PathBuf;

use clap::{Subcommand, ValueEnum};

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
    },
    /// Re-run OCR/VLM/image enrichment for an existing document ID or source path/prefix
    Reprocess {
        /// Document ID, source path, or source path prefix
        target: String,
        /// Do not run semantic indexing after reprocess; run `docs index` later
        #[arg(long)]
        defer_index: bool,
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
    /// Report embedding model and backend status
    ModelStatus,
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

#[derive(Subcommand, Debug, Clone)]
pub enum MeaningAction {
    /// Build meaning records from persisted learning signals
    Build {
        /// Source family to compile from
        #[arg(long, default_value = "learning-events")]
        from: String,
    },
    /// List derived samples
    Samples,
    /// List contrastive pairs
    Contrastive,
    /// List triplets
    Triplets,
    /// Export samples or triplets as JSONL
    Export {
        /// Dataset to export: samples or triplets
        #[arg(long, default_value = "samples")]
        kind: String,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum LearningAction {
    /// Inspect GNN auto-trainer diagnostics
    Gnn {
        #[command(subcommand)]
        action: LearningGnnAction,
    },
    /// Run one autonomous governed-learning proposal/evaluation/apply pass
    Tick,
}

#[derive(Subcommand, Debug, Clone)]
pub enum LearningGnnAction {
    /// Show auto-trainer gates, thresholds, and last-run state
    Status,
}

#[derive(Subcommand, Debug, Clone)]
pub enum MemoryAction {
    /// Re-embed every memory in the graph using the currently-configured
    /// embedding model. Use after swapping models or recovering from a
    /// corrupted prior model. Existing vectors are overwritten in place.
    Reindex {
        /// Confirm a full re-embed (required — implicit guard against
        /// accidentally re-running an expensive operation).
        #[arg(long)]
        all: bool,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum SelfAction {
    /// Extract evidence-backed lessons from a persisted session activity log
    Retrospective {
        /// Session ID under ~/.archon/sessions/<session-id>/activity/events.jsonl
        session_id: String,
        /// Candidate extractor to use
        #[arg(long, value_enum, default_value = "hybrid")]
        analyzer: RetrospectiveAnalyzerArg,
    },
    /// Inspect self-calibration trust records
    Trust {
        #[command(subcommand)]
        action: SelfTrustAction,
    },
    /// Inspect stored plan artifacts and plan-vs-outcome summaries
    Plans {
        #[command(subcommand)]
        action: SelfPlansAction,
    },
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetrospectiveAnalyzerArg {
    /// Run deterministic local rules only
    Heuristic,
    /// Run the configured LLM analyzer only, with local fallback if unavailable
    Llm,
    /// Run deterministic rules plus the configured LLM analyzer
    Hybrid,
}

#[derive(Subcommand, Debug, Clone)]
pub enum SelfTrustAction {
    /// Show domain-scoped self-trust summaries
    Status,
}

#[derive(Subcommand, Debug, Clone)]
pub enum SelfPlansAction {
    /// Compare the latest plan for a session with recorded step outcomes
    Inspect {
        /// Session ID
        session_id: String,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum ConstellationAction {
    /// Build a versioned centroid profile from positive meaning samples
    Build {
        /// Target profile: project, research-domain, or strategic-workflow
        #[arg(long)]
        target: String,
    },
    /// Score text or a file against the latest target centroid
    Score {
        /// Target profile to score against
        #[arg(long, default_value = "project")]
        target: String,
        /// File containing the answer/output to score
        #[arg(long)]
        answer: Option<PathBuf>,
        /// Inline text to score when --answer is not supplied
        #[arg(long)]
        text: Option<String>,
    },
    /// Detect whether text or a file has drifted from the target centroid
    Drift {
        /// Target profile to compare against
        #[arg(long, default_value = "project")]
        target: String,
        /// File containing the answer/output to inspect
        #[arg(long)]
        answer: Option<PathBuf>,
        /// Inline text to inspect when --answer is not supplied
        #[arg(long)]
        text: Option<String>,
        /// Minimum accepted similarity before drift is reported
        #[arg(long, default_value_t = 0.45)]
        threshold: f64,
    },
    /// Bootstrap a centroid profile from recent memories, docs, a session, or an inline file
    Bootstrap {
        /// Target profile to bootstrap: memory, docs, or session
        #[arg(long)]
        target: String,
        /// Maximum source texts to read
        #[arg(long, default_value_t = 50)]
        limit: usize,
        /// Session id for --target session
        #[arg(long)]
        session: Option<String>,
        /// File containing representative texts, one per non-empty line
        #[arg(long)]
        inline_file: Option<PathBuf>,
    },
    /// List persisted constellation centroids
    List,
}

#[derive(Subcommand, Debug)]
pub enum PluginAction {
    /// List all discovered plugins with name, version, and status
    List,
    /// Show detailed information about a plugin
    Info {
        /// Plugin name
        name: String,
    },
}

/// Subcommands for `archon behaviour`
#[derive(Subcommand, Debug)]
pub enum BehaviourAction {
    /// List behaviour proposals (aliases: list, proposals)
    #[command(alias = "list", alias = "proposals")]
    ListProposals {
        /// Show only pending proposals
        #[arg(long)]
        pending: bool,
    },
    /// List learning events (optionally filtered by type)
    ListEvents {
        /// Filter by event type (e.g., FalseCompletionDetected, ManifestApplied)
        #[arg(short, long)]
        event_type: Option<String>,
    },
    /// Show details for a proposal, event, or manifest version
    Show {
        /// ID of the item to show (proposal_id, event_id, or version_id)
        id: String,
    },
    /// Auto-apply a pending proposal (without human review)
    Apply {
        /// Proposal ID to apply
        proposal_id: String,
    },
    /// Show version history for a manifest kind
    History {
        /// Manifest kind (RetrievalProfile, SourceQualityProfile, etc.)
        kind: String,
    },
    /// Generate proposals from recent learning events
    GenerateProposals,
    /// Show learning system status and statistics
    Status,
    /// Approve a pending proposal (human-in-the-loop)
    Approve {
        /// Proposal ID to approve
        proposal_id: String,
    },
    /// Deny a pending proposal
    Deny {
        /// Proposal ID to deny
        proposal_id: String,
    },
    /// Rollback a manifest to a previous version
    Rollback {
        /// Target version ID to rollback to
        version_id: String,
        /// Reason for rollback
        #[arg(short, long)]
        reason: Option<String>,
    },
}
