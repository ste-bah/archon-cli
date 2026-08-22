//! `/clear` — the registry's entry for a command whose body runs upstream.
//!
//! # Where the work happens
//!
//! The clear body is `session_loop::slash_handlers::handle_clear_command`.
//! It cannot live in `ClearHandler::execute`: it locks the agent to save the
//! personality snapshot, fires the `SessionEnd` and `SessionStart` hooks,
//! clears the conversation, and purges the session store — all `.await`s over
//! state `CommandContext` does not carry and the synchronous
//! `CommandHandler::execute` signature cannot reach. So `slash_dispatch`
//! intercepts the command before `handle_slash_command` runs, and this
//! handler exists to give the registry a name, a description and the alias
//! list that `/help` prints.
//!
//! # The gap this module used to leave open
//!
//! The interception was a literal `trimmed == "/clear"`. The alias published
//! right here — `cls` — matched nothing there, so `/cls` fell through to the
//! dispatcher and reached an `execute` whose whole body was `Ok(())`. No
//! clear, no error, no output: the user watched their conversation stay on
//! screen and in storage while believing it was gone.
//!
//! That mattered more than a missing convenience. Clearing is a privacy
//! operation at the store: `delete_all_messages` removes the log *and* the
//! compaction segments, their verbatim bodies, the compaction ledger and
//! every cached projection, because all four otherwise keep readable copies
//! of the cleared conversation — and because segments are addressed by log
//! index, a cleared log restarting at 0 lets survivors re-attach to whatever
//! conversation comes next in that session. `/cls` reached none of it.
//!
//! # What keeps the spellings together now
//!
//! [`command_args`] answers "is this the clear command, and with what
//! arguments?" against [`spellings`], which is [`ClearHandler::NAME`] plus
//! [`ClearHandler::aliases`] — the same alias list `RegistryBuilder::build`
//! indexes, and its only source.
//! `slash_dispatch` asks that question instead of comparing strings, so a
//! third alias added to `aliases()` is intercepted the moment it is declared.
//! `session_loop::slash_dispatch_clear_tests` drives every spelling
//! [`spellings`] reports through the real dispatch and reads the store back,
//! so an alias that stopped reaching the body would fail rather than go
//! quiet.
//!
//! `execute` is consequently unreachable, and says so loudly instead of
//! returning the `Ok(())` that hid the defect for as long as it did.

use crate::command::intercept;
use crate::command::registry::{CommandContext, CommandHandler};

/// Registry entry for `/clear`. The body runs in
/// `session_loop::slash_handlers::handle_clear_command`; see the module docs.
pub(crate) struct ClearHandler;

impl ClearHandler {
    /// The primary spelling, shared by the registry entry in
    /// `registry::default_registry` and by [`command_args`], so the name the
    /// dispatcher knows and the name the interception matches cannot differ.
    pub(crate) const NAME: &'static str = "clear";

    /// Construct a fresh `ClearHandler`. Zero-sized so this is free.
    pub(crate) fn new() -> Self {
        Self
    }
}

impl Default for ClearHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandHandler for ClearHandler {
    fn execute(&self, _ctx: &mut CommandContext, _args: &[String]) -> anyhow::Result<()> {
        // Unreachable: `slash_dispatch` intercepts every spelling
        // `command_args` recognises, which is every spelling the registry
        // resolves to this handler. Reaching here means the interception was
        // removed or bypassed, and the user's clear did not happen. Say so —
        // returning `Ok(())` is what made the `/cls` defect invisible.
        anyhow::bail!(
            "/clear did not run: its body is intercepted in the session loop and \
             the dispatcher was reached instead. Nothing was cleared. This is a \
             dispatch wiring regression — report it."
        )
    }

    fn description(&self) -> &'static str {
        "Clear the current conversation"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &["cls"]
    }
}

/// Every spelling the registry resolves to [`ClearHandler`]: the primary name
/// followed by each published alias.
///
/// Tests iterate this rather than a hand-written list, so a new alias joins
/// the coverage at the moment it is declared.
pub(crate) fn spellings() -> Vec<&'static str> {
    std::iter::once(ClearHandler::NAME)
        .chain(ClearHandler.aliases().iter().copied())
        .collect()
}

/// The argument text after the command name when `input` is `/clear` under any
/// of its [`spellings`]; `None` when `input` is some other command.
///
/// `/clear` takes no arguments, so the returned text is only used to tell a
/// bare command from a non-match — but `/clear please` now clears rather than
/// silently doing nothing, which is the same failure the alias had.
pub(crate) fn command_args(input: &str) -> Option<&str> {
    intercept::command_args(input, &spellings())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use crate::command::dispatcher::Dispatcher;
    use crate::command::registry::RegistryBuilder;

    fn make_ctx() -> (CommandContext, archon_tui::event_channel::TuiEventReceiver) {
        crate::command::test_support::CtxBuilder::new().build()
    }

    #[test]
    fn the_description_and_alias_the_registry_publishes_are_unchanged() {
        let h = ClearHandler::new();
        assert_eq!(h.description(), "Clear the current conversation");
        assert_eq!(h.aliases(), &["cls"]);
        assert_eq!(ClearHandler::NAME, "clear");
    }

    /// The property the interception depends on: every spelling the registry
    /// resolves to `ClearHandler` is a spelling `command_args` claims. A new
    /// alias declared on the handler is covered here without anyone adding a
    /// case, and an alias the interception could not see would fail.
    #[test]
    fn every_spelling_the_registry_resolves_here_is_one_the_interception_claims() {
        let mut b = RegistryBuilder::new();
        b.insert_primary(ClearHandler::NAME, Arc::new(ClearHandler::new()));
        let registry = b.build();

        for spelling in spellings() {
            assert!(
                registry.get(spelling).is_some(),
                "/{spelling} does not resolve in the registry"
            );
            assert_eq!(
                command_args(&format!("/{spelling}")),
                Some(""),
                "/{spelling} resolves to the clear command but the interception \
                 does not recognise it — it would reach the unreachable handler"
            );
        }
    }

    #[test]
    fn another_command_is_not_mistaken_for_clear() {
        assert_eq!(command_args("/compact"), None);
        assert_eq!(command_args("/clearing-house"), None);
    }

    /// Reaching `execute` means the interception did not fire and the user's
    /// conversation was not cleared. That has to be loud.
    #[test]
    fn reaching_the_handler_body_is_an_error_not_a_silent_success() {
        let (mut ctx, _rx) = make_ctx();
        let h = ClearHandler::new();
        let error = h
            .execute(&mut ctx, &[])
            .expect_err("a clear that did not clear must not report success");
        assert!(
            error.to_string().contains("Nothing was cleared"),
            "the error must say the clear did not happen: {error}"
        );
    }

    #[test]
    fn the_dispatcher_surfaces_that_error_to_the_user() {
        let mut b = RegistryBuilder::new();
        b.insert_primary(ClearHandler::NAME, Arc::new(ClearHandler::new()));
        let dispatcher = Dispatcher::new(Arc::new(b.build()));

        for spelling in spellings() {
            let (mut ctx, mut rx) = make_ctx();
            assert!(
                dispatcher
                    .dispatch(&mut ctx, &format!("/{spelling}"))
                    .is_err(),
                "/{spelling} reaching the dispatcher must not look like success"
            );
            let events = crate::command::test_support::drain_tui_events(&mut rx);
            assert!(
                events.iter().any(|event| matches!(
                    event,
                    archon_tui::app::TuiEvent::Error(message)
                        if message.contains("Nothing was cleared")
                )),
                "the user must be told the clear did not run, got: {events:?}"
            );
        }
    }
}
