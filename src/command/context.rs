//! TASK-AGS-807: async `CommandContext` builder (snapshot pattern).
//!
//! `CommandHandler::execute` is SYNC (Q1=A invariant). The shipped
//! `/status` body relies on four `tokio::sync::Mutex` guards acquired via
//! `.lock().await`. To bridge the gap, this facade delegates construction,
//! effects, and canonical-primary resolution to focused private modules while
//! preserving the crate-visible dispatch entry points.

mod builder;
mod effects;
mod primary;
#[cfg(test)]
mod slash_ctx_fixture;
#[cfg(test)]
mod tests;

pub(crate) use builder::build_command_context;
pub(crate) use effects::apply_effect;
pub(crate) use primary::resolve_primary_from_input;
