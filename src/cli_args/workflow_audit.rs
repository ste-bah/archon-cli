//! Audit controls never imply workflow launch or noninteractive approval.
use clap::Subcommand;
use serde::{Deserialize, Serialize};

#[derive(Subcommand, Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum AuditAction {
    /// Inspect persisted policy, consumption and findings
    Status { run_id: String },
    /// Add allowance without resetting consumption
    ExtendBudget {
        run_id: String,
        #[arg(long)]
        extra_refreshes: Option<u64>,
        #[arg(long)]
        extra_seconds: Option<u64>,
        #[arg(long)]
        reason: String,
    },
    /// Replace selected limits; positive integers or unlimited
    SetBudget {
        run_id: String,
        #[arg(long)]
        attempt_timeout_secs: Option<String>,
        #[arg(long)]
        total_time_secs: Option<String>,
        #[arg(long)]
        unexpected_change_refreshes: Option<String>,
        #[arg(long)]
        reason: String,
    },
    /// Queue one review of a disputed finding; does not launch a run
    Reassess {
        run_id: String,
        #[arg(long)]
        finding: String,
        #[arg(long)]
        snapshot: String,
        #[arg(long)]
        reason: String,
    },
    /// Record an exception for exactly one finding and snapshot
    Waive {
        run_id: String,
        #[arg(long)]
        finding: String,
        #[arg(long)]
        snapshot: String,
        #[arg(long)]
        reason: String,
    },
}
impl AuditAction {
    pub(crate) fn run_id(&self) -> &str {
        match self {
            Self::Status{run_id} | Self::ExtendBudget{run_id,..} | Self::SetBudget{run_id,..}
            | Self::Reassess{run_id,..} | Self::Waive{run_id,..} => run_id,
        }
    }
}
