use std::path::PathBuf;

use clap::Subcommand;

#[derive(Subcommand, Debug, Clone)]
pub enum WorldAction {
    /// Show local world-model status and cold-start gates
    Status,
    /// Ingest one session or backfill the local world-model corpus
    Ingest(WorldIngestArgs),
    /// Ask the local world model for a fail-open next-state advisory
    PredictNext(WorldPredictNextArgs),
    /// Score alternate actions with the local counterfactual advisor
    ScoreActions(WorldScoreActionsArgs),
    /// Explain a persisted world-model prediction
    Explain(WorldExplainArgs),
    /// Attach the observed outcome for a persisted prediction
    RecordOutcome(WorldRecordOutcomeArgs),
    /// Train a local CPU candidate from the stored world-model corpus
    Train(WorldTrainArgs),
    /// Train a JEPA-inspired representation candidate from the stored world-model corpus
    TrainJepa(WorldTrainJepaArgs),
    /// Run one idle-aware dynamic trainer tick
    TrainerTick(WorldTrainerTickArgs),
    /// Evaluate a candidate checkpoint against promotion gates
    Eval(WorldEvalArgs),
    /// Evaluate a JEPA-inspired candidate against promotion gates
    EvalJepa(WorldEvalJepaArgs),
    /// Show status of a JEPA eval run
    EvalJepaStatus(WorldEvalJepaStatusArgs),
    /// List recent JEPA eval runs
    EvalJepaRuns(WorldEvalJepaRunsArgs),
    /// Cancel a running JEPA eval job
    EvalJepaCancel(WorldEvalJepaCancelArgs),
    /// Inspect a JEPA-inspired candidate manifest and gate state
    InspectJepa(WorldInspectJepaArgs),
    /// Compare JEPA-inspired representations against an exploratory baseline
    CompareRepresentations(WorldCompareRepresentationsArgs),
    /// Promote a candidate checkpoint as advisory active
    Promote(WorldPromoteArgs),
    /// Promote a JEPA-inspired candidate after promotion gates pass
    PromoteJepa(WorldPromoteJepaArgs),
    /// Roll back the active advisory pointer to a prior model
    Rollback(WorldRollbackArgs),
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
    /// Approve a guarded action despite unresolved risk
    Approve {
        /// Guarded action id
        action_id: String,
        /// Required approval reason
        #[arg(long)]
        reason: String,
    },
    /// Mark one verification requirement as intentionally skipped
    SkipVerification {
        /// Verification requirement id
        requirement_id: String,
        /// Required skip reason
        #[arg(long)]
        reason: String,
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

#[path = "world_model_actions_args.rs"]
pub(super) mod world_model_actions_args;
pub use world_model_actions_args::*;
