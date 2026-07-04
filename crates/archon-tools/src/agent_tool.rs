mod catalog;
mod core;
mod failure;
mod request;
mod run;

#[cfg(test)]
mod tests;

pub use crate::subagent_request::SubagentRequest;
pub use catalog::AgentCatalogTool;
pub use core::AgentTool;
pub use failure::classify_failure_prefix;
pub use request::AgentToolError;
pub use run::{run_subagent, run_subagent_foreground};

#[cfg(test)]
pub(crate) use core::AGENT_DESCRIPTION_LIMIT_BYTES;
