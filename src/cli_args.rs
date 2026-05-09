//! CLI argument definitions for the `archon` binary.
//!
//! The public `cli_args` module is kept as a compatibility shell while
//! clap definitions live in focused submodules.

mod agent_actions;
mod auth;
mod commands;
mod data_actions;
mod root;
mod strategy_actions;

pub use agent_actions::{AgentAction, AgentEvolveAction};
pub use auth::{AuthArgs, AuthProviderKind, AuthSubcommand, ChatArgs};
pub use commands::Commands;
pub use data_actions::{
    BehaviourAction, ConstellationAction, DocsAction, KbAction, LearningAction, LearningGnnAction,
    MeaningAction, MemoryAction, PluginAction, ProvAction, RemoteAction, RetrospectiveAnalyzerArg,
    SelfAction, SelfPlansAction, SelfTrustAction,
};
pub use root::Cli;
pub use strategy_actions::{
    CompletionAction, GametheoryAction, PipelineAction, ProviderProfilesAction, ProvidersAction,
    SandboxAction, TeamAction,
};

#[cfg(test)]
mod tests;
