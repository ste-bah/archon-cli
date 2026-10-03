//! Argument structs for `AgentEvolveAction` variants. Each struct gets its own
//! clap `augment_args` frame, so the frames do not stack (#233).

use clap::Args;

#[derive(Args, Debug, Clone)]
pub struct AgentEvolveActiveArgs {
    /// Agent type to inspect
    #[arg(long)]
    pub agent: String,
    /// Output the full Cozo record as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug, Clone)]
pub struct AgentEvolveApplyArgs {
    /// Agent evolution proposal ID
    pub proposal_id: String,
    /// Mark the created profile version active in Cozo
    #[arg(long)]
    pub activate: bool,
}

#[derive(Args, Debug, Clone)]
pub struct AgentEvolveApproveArgs {
    /// Agent evolution proposal ID
    pub proposal_id: String,
}

#[derive(Args, Debug, Clone)]
pub struct AgentEvolveDigestArgs {
    /// Agent type to inspect
    #[arg(long)]
    pub agent: String,
    /// Persist generated claims into Cozo learning events
    #[arg(long)]
    pub persist: bool,
    /// Output the digest as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug, Clone)]
pub struct AgentEvolveGenerateArgs {
    /// Agent type to scan in the Cozo-backed performance ledger
    #[arg(long)]
    pub agent: String,
}

#[derive(Args, Debug, Clone)]
pub struct AgentEvolveHistoryArgs {
    /// Agent type to inspect
    #[arg(long)]
    pub agent: String,
    /// Output history as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug, Clone)]
pub struct AgentEvolveInspectArgs {
    /// Agent evolution proposal ID
    pub proposal_id: String,
    /// Output the full inspection as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug, Clone)]
pub struct AgentEvolveListArgs {
    /// Filter by proposal status, e.g. pending, rejected, approved
    #[arg(long)]
    pub status: Option<String>,
    /// Filter by agent type
    #[arg(long)]
    pub agent: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct AgentEvolveMemoryCandidatesArgs {
    /// Agent type to inspect
    #[arg(long)]
    pub agent: String,
}

#[derive(Args, Debug, Clone)]
pub struct AgentEvolveMemoryPromoteArgs {
    /// Memory promotion candidate ID
    pub candidate_id: String,
    /// Minimum weighted score required for promotion
    #[arg(long, default_value_t = 0.85)]
    pub min_score: f64,
    /// Show what would be written without storing memory
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Args, Debug, Clone)]
pub struct AgentEvolvePermissionsArgs {
    /// Agent evolution proposal ID
    pub proposal_id: String,
    /// Output the full permission review as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug, Clone)]
pub struct AgentEvolveRejectArgs {
    /// Agent evolution proposal ID
    pub proposal_id: String,
}

#[derive(Args, Debug, Clone)]
pub struct AgentEvolveReportArgs {
    /// Agent type to inspect
    #[arg(long)]
    pub agent: String,
    /// Output the report as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug, Clone)]
pub struct AgentEvolveStatusArgs {
    /// Agent type to inspect
    #[arg(long)]
    pub agent: String,
    /// Output status as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug, Clone)]
pub struct AgentEvolveShadowArgs {
    /// Agent evolution proposal ID
    pub proposal_id: String,
    /// Optional archived task set or evaluation suite identifier
    #[arg(long)]
    pub task_set: Option<String>,
    /// Output the persisted shadow evaluation as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug, Clone)]
pub struct AgentEvolveRollbackArgs {
    /// Agent type that owns the profile version
    #[arg(long)]
    pub agent: String,
    /// Existing profile version ID to restore from
    pub version_id: String,
    /// Mark the rollback profile version active in Cozo
    #[arg(long)]
    pub activate: bool,
}
