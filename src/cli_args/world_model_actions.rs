use std::path::PathBuf;

use clap::Subcommand;

#[derive(Subcommand, Debug, Clone)]
pub enum WorldAction {
    /// Show local world-model status and cold-start gates
    Status,
    /// Ingest one session or backfill the local world-model corpus
    Ingest {
        /// Session ID to ingest
        session_id: Option<String>,
        /// Backfill all available sessions, activity logs, pipeline bundles, and transcripts
        #[arg(long)]
        backfill: bool,
    },
    /// Ask the local world model for a fail-open next-state advisory
    PredictNext {
        /// Session ID for this advisory
        #[arg(long)]
        session_id: String,
        /// Stable action reference for event correlation
        #[arg(long)]
        action_ref: String,
        /// Short action summary to score
        #[arg(long)]
        summary: String,
    },
    /// Score alternate actions with the local counterfactual advisor
    ScoreActions {
        /// Task context to score against
        #[arg(long)]
        task: String,
        /// JSON file containing an array of candidate actions
        #[arg(long)]
        actions: PathBuf,
    },
    /// Explain a persisted world-model prediction
    Explain {
        /// Prediction id to inspect
        prediction_id: String,
    },
    /// Attach the observed outcome for a persisted prediction
    RecordOutcome {
        /// Prediction id to update
        prediction_id: String,
        /// Redacted actual next-state summary
        #[arg(long)]
        actual_summary: String,
    },
    /// Train a local CPU candidate from the stored world-model corpus
    Train {
        /// Write a candidate checkpoint instead of touching the active model
        #[arg(long, default_value_t = true)]
        candidate: bool,
        /// Override max runtime for this training invocation
        #[arg(long)]
        max_runtime_ms: Option<u64>,
    },
    /// Train a JEPA representation candidate from the stored world-model corpus
    TrainJepa {
        /// Write a candidate checkpoint instead of touching the active model
        #[arg(long, default_value_t = true)]
        candidate: bool,
        /// Override max runtime for this training invocation
        #[arg(long)]
        max_runtime_ms: Option<u64>,
    },
    /// Run one idle-aware dynamic trainer tick
    TrainerTick {
        /// Age of the latest foreground activity in milliseconds
        #[arg(long)]
        last_activity_age_ms: Option<u64>,
        /// Age of the latest world-model training run in milliseconds
        #[arg(long)]
        last_training_age_ms: Option<u64>,
        /// Current battery percentage, when known
        #[arg(long)]
        battery_percent: Option<u8>,
        /// Treat the machine as unplugged for battery gating
        #[arg(long)]
        unplugged: bool,
    },
    /// Evaluate a candidate checkpoint against promotion gates
    Eval {
        /// Candidate model id to inspect
        candidate_id: Option<String>,
    },
    /// Evaluate a JEPA candidate against JEPA-specific promotion gates
    EvalJepa {
        /// Candidate model id to inspect
        candidate_id: String,
    },
    /// Inspect a JEPA candidate manifest and gate state
    InspectJepa {
        /// Candidate model id to inspect
        candidate_id: String,
    },
    /// Compare JEPA representations against an exploratory baseline
    CompareRepresentations {
        /// Exploratory baseline backend. Promotion gating always uses fastembed.
        #[arg(long, default_value = "fastembed")]
        baseline: String,
        /// JEPA candidate model id to compare
        #[arg(long)]
        candidate: String,
    },
    /// Promote a candidate checkpoint as advisory active
    Promote {
        /// Candidate model id to promote
        model_id: String,
    },
    /// Promote a JEPA candidate after JEPA-specific gates pass
    PromoteJepa {
        /// JEPA candidate model id to promote
        model_id: String,
    },
    /// Roll back the active advisory pointer to a prior model
    Rollback {
        /// Prior model id to restore
        model_id: String,
    },
    /// Inspect and configure runtime world-model guardrails
    Guard {
        #[command(subcommand)]
        action: WorldGuardAction,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum WorldGuardAction {
    /// Show runtime guardrail status and ledger counters
    Status,
    /// Inspect one guarded action
    Inspect {
        /// Guarded action id
        action_id: String,
    },
    /// List recently guarded actions
    List {
        /// Filter by session id
        #[arg(long)]
        session: Option<String>,
        /// Filter by surface name
        #[arg(long)]
        surface: Option<String>,
        /// Filter by status: all, blocked, open, complete
        #[arg(long)]
        status: Option<String>,
    },
    /// Replay structured guardrail outcomes into downstream stores
    ReplayOutcomes {
        /// Filter by session id
        #[arg(long)]
        session: Option<String>,
    },
    /// Show or update guardrail policy
    Policy {
        #[command(subcommand)]
        action: WorldGuardPolicyAction,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum WorldGuardPolicyAction {
    /// Show active guardrail policy
    Show,
    /// Persist selected guardrail policy modes to config.toml
    Set {
        /// Desired interactive mode
        #[arg(long)]
        interactive_mode: Option<String>,
        /// Desired pipeline mode
        #[arg(long)]
        pipeline_mode: Option<String>,
    },
}
