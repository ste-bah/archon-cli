//! Agent-facing CLI actions: meaning, learning, memory, style, self, constellation,
//! plugin and behaviour subcommands.
//!
//! Split out of `data_actions.rs` to keep both files under the 500-line gate.

use std::path::PathBuf;

use clap::{Subcommand, ValueEnum};
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
pub enum StyleAction {
    /// Train an output-style from sample prose by measuring its Lanham style
    /// (POS-free, fully offline). Writes a `.md` Archon output-style.
    Train {
        /// Sample text file(s) to learn the style from (omit to read stdin)
        files: Vec<String>,
        /// Name for the output-style (basename of the `.md` + style id)
        #[arg(long, default_value = "trained-style")]
        name: String,
        /// Genre register frame (academic, narrative, journalistic, technical, general)
        #[arg(long, default_value = "academic")]
        genre: String,
        /// Output path (default: ~/.archon/output-styles/<name>.md)
        #[arg(long)]
        out: Option<String>,
        /// Print the rendered output-style to stdout instead of writing a file
        #[arg(long)]
        stdout: bool,
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
