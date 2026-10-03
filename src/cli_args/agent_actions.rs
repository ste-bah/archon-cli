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
    Active(AgentEvolveActiveArgs),
    /// Apply an approved proposal into a governed profile version
    Apply(AgentEvolveApplyArgs),
    /// Mark an agent evolution proposal as approved for later apply
    Approve(AgentEvolveApproveArgs),
    /// Compile an Archon-native knowledge digest for an agent
    Digest(AgentEvolveDigestArgs),
    /// Generate governed proposals from persisted agent performance ledger rows
    Generate(AgentEvolveGenerateArgs),
    /// Show governed profile version history for an agent
    History(AgentEvolveHistoryArgs),
    /// Inspect one Cozo-backed agent evolution proposal
    Inspect(AgentEvolveInspectArgs),
    /// List Cozo-backed agent evolution proposals
    List(AgentEvolveListArgs),
    /// List Cozo-backed memory promotion candidates for an agent
    MemoryCandidates(AgentEvolveMemoryCandidatesArgs),
    /// Promote one memory candidate into the Archon memory graph
    MemoryPromote(AgentEvolveMemoryPromoteArgs),
    /// Show permission-impact details for one proposal
    Permissions(AgentEvolvePermissionsArgs),
    /// Reject an agent evolution proposal
    Reject(AgentEvolveRejectArgs),
    /// Summarize governed evolution state for an agent
    Report(AgentEvolveReportArgs),
    /// Show concise governed evolution status for an agent
    Status(AgentEvolveStatusArgs),
    /// Record a Cozo-backed shadow evaluation for one proposal
    Shadow(AgentEvolveShadowArgs),
    /// Create a rollback profile version from an existing profile version
    Rollback(AgentEvolveRollbackArgs),
}

#[path = "agent_actions_args.rs"]
pub(super) mod agent_actions_args;
pub use agent_actions_args::*;
