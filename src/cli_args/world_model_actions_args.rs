//! Argument structs for `WorldAction` variants. Each struct gets its own
//! clap `augment_args` frame, so the frames do not stack (#233).

use clap::Args;

use super::*;

#[derive(Args, Debug, Clone)]
pub struct WorldIngestArgs {
    /// Session ID to ingest
    pub session_id: Option<String>,
    /// Backfill all available sessions, activity logs, pipeline bundles, and transcripts
    #[arg(long)]
    pub backfill: bool,
}

#[derive(Args, Debug, Clone)]
pub struct WorldPredictNextArgs {
    /// Session ID for this advisory
    #[arg(long)]
    pub session_id: String,
    /// Stable action reference for event correlation
    #[arg(long)]
    pub action_ref: String,
    /// Short action summary to score
    #[arg(long)]
    pub summary: String,
}

#[derive(Args, Debug, Clone)]
pub struct WorldScoreActionsArgs {
    /// Task context to score against
    #[arg(long)]
    pub task: String,
    /// JSON file containing an array of candidate actions
    #[arg(long)]
    pub actions: PathBuf,
}

#[derive(Args, Debug, Clone)]
pub struct WorldExplainArgs {
    /// Prediction id to inspect
    pub prediction_id: String,
}

#[derive(Args, Debug, Clone)]
pub struct WorldRecordOutcomeArgs {
    /// Prediction id to update
    pub prediction_id: String,
    /// Redacted actual next-state summary
    #[arg(long)]
    pub actual_summary: String,
}

#[derive(Args, Debug, Clone)]
pub struct WorldTrainArgs {
    /// Write a candidate checkpoint instead of touching the active model
    #[arg(long, default_value_t = true)]
    pub candidate: bool,
    /// Override max runtime for this training invocation
    #[arg(long)]
    pub max_runtime_ms: Option<u64>,
}

#[derive(Args, Debug, Clone)]
pub struct WorldTrainJepaArgs {
    /// Write a candidate checkpoint instead of touching the active model
    #[arg(long, default_value_t = true)]
    pub candidate: bool,
    /// Override max runtime for this training invocation
    #[arg(long)]
    pub max_runtime_ms: Option<u64>,
}

#[derive(Args, Debug, Clone)]
pub struct WorldTrainerTickArgs {
    /// Age of the latest foreground activity in milliseconds
    #[arg(long)]
    pub last_activity_age_ms: Option<u64>,
    /// Age of the latest world-model training run in milliseconds
    #[arg(long)]
    pub last_training_age_ms: Option<u64>,
    /// Current battery percentage, when known
    #[arg(long)]
    pub battery_percent: Option<u8>,
    /// Treat the machine as unplugged for battery gating
    #[arg(long)]
    pub unplugged: bool,
}

#[derive(Args, Debug, Clone)]
pub struct WorldEvalArgs {
    /// Candidate model id to inspect
    pub candidate_id: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct WorldEvalJepaArgs {
    /// Candidate model id to evaluate
    pub candidate_id: String,

    /// Run full promotion-grade evaluation.
    /// Without this flag, eval-jepa uses quick Tier-0 mode and may skip the baseline.
    #[arg(long)]
    pub full: bool,

    /// Request background evaluation.
    /// Parsed for compatibility; the current CLI returns a clear deferral error.
    #[arg(long)]
    pub background: bool,

    /// Inspect resume preconditions for a previously paused eval run by its run-id
    #[arg(long)]
    pub resume: Option<String>,

    /// Force a specific backend: cpu, metal, cuda.
    /// Parsed for compatibility; currently warns and uses the candidate/config path.
    #[arg(long, value_parser = ["cpu", "metal", "cuda"])]
    pub backend: Option<String>,

    /// Skip embedding cache reads and writes for this run.
    /// Parsed for compatibility; currently warns unless validating resume preconditions.
    #[arg(long)]
    pub no_cache: bool,
}

#[derive(Args, Debug, Clone)]
pub struct WorldEvalJepaStatusArgs {
    /// Run ID (e.g. jeval-...)
    pub run_id: String,
}

#[derive(Args, Debug, Clone)]
pub struct WorldEvalJepaRunsArgs {
    /// Maximum number of runs to show
    #[arg(long, default_value = "10")]
    pub limit: usize,
}

#[derive(Args, Debug, Clone)]
pub struct WorldEvalJepaCancelArgs {
    /// Run ID to cancel
    pub run_id: String,
}

#[derive(Args, Debug, Clone)]
pub struct WorldInspectJepaArgs {
    /// Candidate model id to inspect
    pub candidate_id: String,
}

#[derive(Args, Debug, Clone)]
pub struct WorldCompareRepresentationsArgs {
    /// Exploratory baseline backend. Promotion gating always uses fastembed.
    #[arg(long, default_value = "fastembed")]
    pub baseline: String,
    /// JEPA-inspired candidate model id to compare
    #[arg(long)]
    pub candidate: String,
}

#[derive(Args, Debug, Clone)]
pub struct WorldPromoteArgs {
    /// Candidate model id to promote
    pub model_id: String,
}

#[derive(Args, Debug, Clone)]
pub struct WorldPromoteJepaArgs {
    /// JEPA-inspired candidate model id to promote
    pub model_id: String,
}

#[derive(Args, Debug, Clone)]
pub struct WorldRollbackArgs {
    /// Prior model id to restore
    pub model_id: String,
}
