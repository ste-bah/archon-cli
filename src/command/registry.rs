//! Slash command registry.
//!
//! Public command-registry paths are preserved here while the
//! implementation lives in focused submodules.

mod context;
mod default;
mod effect;
mod handler;
mod table;

pub(crate) use context::CommandContext;
pub(crate) use default::default_registry;
pub(crate) use effect::CommandEffect;
pub(crate) use handler::CommandHandler;
pub(crate) use table::{Registry, RegistryBuilder};

#[cfg(test)]
mod tests;
