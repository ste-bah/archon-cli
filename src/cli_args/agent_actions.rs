use clap::Subcommand;

#[derive(Subcommand, Debug, Clone)]
pub enum AgentAction {
    /// Inspect governed agent profile evolution
    Evolve {
        #[command(subcommand)]
        action: AgentEvolveAction,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum AgentEvolveAction {
    /// Show the active governed profile version for an agent
    Active {
        /// Agent type to inspect
        #[arg(long)]
        agent: String,
        /// Output the full Cozo record as JSON
        #[arg(long)]
        json: bool,
    },
    /// Apply an approved proposal into a governed profile version
    Apply {
        /// Agent evolution proposal ID
        proposal_id: String,
        /// Mark the created profile version active in Cozo
        #[arg(long)]
        activate: bool,
    },
    /// Mark an agent evolution proposal as approved for later apply
    Approve {
        /// Agent evolution proposal ID
        proposal_id: String,
    },
    /// Generate governed proposals from persisted agent performance ledger rows
    Generate {
        /// Agent type to scan in the Cozo-backed performance ledger
        #[arg(long)]
        agent: String,
    },
    /// Inspect one Cozo-backed agent evolution proposal
    Inspect {
        /// Agent evolution proposal ID
        proposal_id: String,
        /// Output the full inspection as JSON
        #[arg(long)]
        json: bool,
    },
    /// List Cozo-backed agent evolution proposals
    List {
        /// Filter by proposal status, e.g. pending, rejected, approved
        #[arg(long)]
        status: Option<String>,
        /// Filter by agent type
        #[arg(long)]
        agent: Option<String>,
    },
    /// List Cozo-backed memory promotion candidates for an agent
    MemoryCandidates {
        /// Agent type to inspect
        #[arg(long)]
        agent: String,
    },
    /// Promote one memory candidate into the Archon memory graph
    MemoryPromote {
        /// Memory promotion candidate ID
        candidate_id: String,
        /// Minimum weighted score required for promotion
        #[arg(long, default_value_t = 0.85)]
        min_score: f64,
        /// Show what would be written without storing memory
        #[arg(long)]
        dry_run: bool,
    },
    /// Show permission-impact details for one proposal
    Permissions {
        /// Agent evolution proposal ID
        proposal_id: String,
    },
    /// Reject an agent evolution proposal
    Reject {
        /// Agent evolution proposal ID
        proposal_id: String,
    },
    /// Summarize governed evolution state for an agent
    Report {
        /// Agent type to inspect
        #[arg(long)]
        agent: String,
        /// Output the report as JSON
        #[arg(long)]
        json: bool,
    },
    /// Record a Cozo-backed shadow evaluation for one proposal
    Shadow {
        /// Agent evolution proposal ID
        proposal_id: String,
        /// Optional archived task set or evaluation suite identifier
        #[arg(long)]
        task_set: Option<String>,
        /// Output the persisted shadow evaluation as JSON
        #[arg(long)]
        json: bool,
    },
    /// Create a rollback profile version from an existing profile version
    Rollback {
        /// Agent type that owns the profile version
        #[arg(long)]
        agent: String,
        /// Existing profile version ID to restore from
        version_id: String,
        /// Mark the rollback profile version active in Cozo
        #[arg(long)]
        activate: bool,
    },
}
