//! Provider-neutral dynamic workflow runtime for Archon.

pub mod command;
pub mod config;
pub mod context;
pub mod error;
pub mod events;
mod exec_state;
pub mod executor;
pub mod fanout;
mod generated;
pub mod learning;
pub mod lifecycle;
pub mod planner;
pub mod policy;
pub mod provider_tiers;
pub mod quality_gate;
pub mod reducers;
mod request;
pub mod run;
pub mod runner;
pub mod spec;
mod spec_deser;
pub mod stage;
pub mod store;
pub mod template;
pub mod tui_events;
pub mod web_api;

pub use command::{CommandAction, WorkflowCommand};
pub use config::WorkflowConfig;
pub use error::{WorkflowError, WorkflowResult};
pub use events::{
    CompactProgress, WorkflowEvent, WorkflowEventKind, WorkflowEventLog, contains_forbidden_field,
};
pub use executor::{ExecutionReport, WorkflowExecutor};
pub use learning::{
    Verification, WorkflowLearningRecord, WorkflowLearningSink, WorkflowRunLearningSummary,
    learning_records,
};
pub use lifecycle::{LifecycleAction, LifecycleController};
pub use planner::{HeuristicWorkflowPlanner, WorkflowPlanner};
pub use policy::{PolicyDecision, WorkflowPolicy};
pub use provider_tiers::{
    ProviderFamily, ProviderTierResolver, ResolvedProviderTier, classify_provider,
};
pub use quality_gate::{ForcedAcceptAudit, QualityGate, QualityGateDecision};
pub use reducers::{ReducerInput, ReducerOutput, ReducerRegistry};
pub use run::{ArtifactRef, RunStatus, StageStatus, WorkflowRun};
pub use runner::{DeterministicStageRunner, StageRunOutput, StageRunRequest, WorkflowStageRunner};
pub use spec::{
    ArtifactPolicy, ProviderTier, ReducerKind, RetryPolicy, StageKind, StageSpec, WorkflowSpec,
};
pub use store::WorkflowStore;
pub use template::{SavedWorkflowTemplate, TemplateRegistry};
