use clap::Subcommand;

#[derive(Subcommand, Debug, Clone)]
pub enum ReasoningAction {
    /// Show reasoning-quality store, shadow, critic, and dead-letter status
    Status,
    /// Inspect reasoning-quality events for one session
    Inspect {
        session_id: String,
        /// Analyzer mode used for post-session inspection
        #[arg(long, default_value = "heuristic")]
        analyzer: String,
    },
    /// Deterministically backfill reasoning-quality rows from historical sessions
    Backfill {
        /// Limit number of sessions to scan
        #[arg(long)]
        sessions: Option<usize>,
        /// Also emit world-model rows
        #[arg(long)]
        emit_world_rows: bool,
        /// Allow LLM critic during backfill; requires policy and cost confirmation
        #[arg(long)]
        include_llm: bool,
    },
    /// List claim events for one session
    Claims { session_id: String },
    /// Show repeated reasoning-failure pattern candidates
    Patterns,
    /// Inspect critic cost and budget state
    Cost {
        #[command(subcommand)]
        action: ReasoningCostAction,
    },
    /// Replay failed bridge events from the dead-letter ledger
    ReplayDeadLetter {
        /// Restrict replay to one bridge name
        #[arg(long)]
        bridge: Option<String>,
    },
    /// Report shadow-mode fixture and operator-label precision state
    ShadowReport,
    /// Interactively label sampled real-session claims
    SampleLabel {
        session_id: String,
        /// Restrict labeling to one turn
        #[arg(long)]
        turn: Option<u64>,
    },
    /// Migrate reasoning-quality schema state
    Migrate {
        /// Target schema version
        #[arg(long = "to-version")]
        to_version: u32,
        /// Report planned mutations without writing
        #[arg(long)]
        dry_run: bool,
    },
    /// Audit labeled extractor fixtures for PII/secrets
    FixtureAudit,
}

#[derive(Subcommand, Debug, Clone)]
pub enum ReasoningCostAction {
    /// Show critic budget status
    Status,
}

#[derive(Subcommand, Debug, Clone)]
pub enum BriefingAction {
    /// Preview proactive session briefing content
    Preview {
        /// Optional task hint used for relevance ranking
        #[arg(long)]
        task: Option<String>,
    },
}
