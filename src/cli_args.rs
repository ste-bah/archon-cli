//! CLI argument definitions for the `archon` binary.
//!
//! The public `cli_args` module is kept as a compatibility shell while
//! clap definitions live in focused submodules.

mod agent_actions;
mod auth;
mod cognitive_actions;
mod commands;
mod data_actions;
mod permissions_actions;
mod reasoning_actions;
mod root;
mod strategy_actions;
mod video_actions;
mod world_model_actions;

pub use agent_actions::{AgentAction, AgentEvolveAction};
pub use auth::{AuthArgs, AuthProviderKind, AuthSubcommand, ChatArgs};
pub use cognitive_actions::{CognitiveAction, CognitiveDaemonAction};
pub use commands::Commands;
pub use data_actions::{
    BehaviourAction, ConstellationAction, DocsAction, DocsIndexDaemonAction, KbAction,
    LearningAction, LearningGnnAction, MeaningAction, MemoryAction, PluginAction, ProvAction,
    RemoteAction, RetrospectiveAnalyzerArg, SelfAction, SelfPlansAction, SelfTrustAction,
};
pub use permissions_actions::PermissionsAction;
pub use reasoning_actions::{BriefingAction, ReasoningAction, ReasoningCostAction};
pub use root::Cli;
pub use strategy_actions::{
    CompletionAction, GametheoryAction, PipelineAction, ProviderProfilesAction, ProvidersAction,
    SandboxAction, TeamAction, WorkflowAction,
};
pub use video_actions::VideoAction;
pub use world_model_actions::{WorldAction, WorldGuardAction, WorldGuardPolicyAction};

#[cfg(test)]
mod permissions_parse_tests;
#[cfg(test)]
mod sandbox_parse_tests;
#[cfg(test)]
mod tests;
