//! `/config` — the registry's entry for a command whose body runs upstream.
//!
//! # Where the work happens
//!
//! [`handle_config_command`] below does the real work. It is `async` and
//! reads the whole `SlashCommandContext`, neither of which the synchronous
//! `CommandHandler::execute` signature can reach, so `command::slash`
//! intercepts the command before the dispatcher runs and calls it directly.
//! `ConfigHandler` exists to give the registry a name, a description and the
//! alias list `/help` prints.
//!
//! # The gap this module used to leave open
//!
//! The interception matched the literal `/config` only. The aliases declared
//! right here — `settings` and `prefs` — reached the dispatcher instead, and
//! `execute` was `Ok(())`: typing `/settings` printed nothing and changed
//! nothing, with no error to say which of the two had happened. The same
//! shape as `/cls` against `/clear`, and found while fixing it.
//!
//! [`command_args`] now answers "is this the config command, and with what
//! arguments?" from [`ConfigHandler::NAME`] plus [`ConfigHandler::aliases`] —
//! the same list `RegistryBuilder::build` indexes. An alias added to the
//! handler is intercepted by construction. `execute` is consequently
//! unreachable and says so rather than returning a success it did not earn.

use crate::command::intercept;
use crate::command::registry::{CommandContext, CommandHandler};
use crate::slash_context::SlashCommandContext;
use archon_tui::app::TuiEvent;

/// Handle `/config` commands: list, get, set.
///
/// TASK-SESSION-LOOP-EXTRACT: returns an explicit
/// `Pin<Box<dyn Future + Send + 'a>>` rather than an inferred
/// `async fn` future. Called from the Box::pin'd
/// `handle_slash_command` inside `session_loop::run_session_loop`;
/// an explicit Send bound avoids rustc's HRTB inference failure
/// across the `&str` / `&SlashCommandContext` borrows held over
/// awaits (rust-lang/rust#102211). The A-2 channel flip fixed the
/// `&Sender<TuiEvent>` HRTB variant; the other borrow variants
/// remain, hence the explicit Pin<Box<..>>.
pub fn handle_config_command<'a>(
    args: &'a str,
    tui_tx: &'a archon_tui::event_channel::TuiEventSender,
    ctx: &'a SlashCommandContext,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
    Box::pin(async move {
        // The caller has already stripped the command name — under whichever
        // spelling the user typed — so what arrives here is just the argument
        // text. Parsing `/config` back out of the input is what left
        // `/settings` and `/prefs` unable to carry arguments at all.
        let parts: Vec<&str> = args.trim().splitn(2, ' ').collect();
        let key = parts.first().map(|s| s.trim()).unwrap_or("");
        let value = parts.get(1).map(|s| s.trim()).unwrap_or("");

        // #189 Phase 7: the effective configuration in one place. `sources`
        // says where settings came from; this says what they resolved to and
        // which ARCHON_* variables are actually set in this process.
        if key == "dump" {
            let output = match super::config_dump::dump() {
                Ok(text) => format!("\n{text}"),
                Err(error) => format!("\nCould not dump configuration: {error}\n"),
            };
            let _ = tui_tx.send_async(TuiEvent::TextDelta(output)).await;
            return;
        }

        if key == "sources" {
            let output = archon_core::config_source::format_sources(&ctx.config_sources);
            if output.is_empty() {
                let _ = tui_tx
                    .send_async(TuiEvent::TextDelta("\nNo config sources tracked.\n".into()))
                    .await;
            } else {
                let _ = tui_tx
                    .send_async(TuiEvent::TextDelta(format!("\nConfig sources:\n{output}")))
                    .await;
            }
            return;
        }

        if key.is_empty() {
            // List all config keys with current values
            let keys = archon_tools::config_tool::all_keys();
            let mut lines = String::from("\nRuntime configuration:\n");
            for k in &keys {
                let val = archon_tools::config_tool::get_config_value(k)
                    .unwrap_or_else(|| "(unknown)".into());
                lines.push_str(&format!("  {k} = {val}\n"));
            }
            let _ = tui_tx.send_async(TuiEvent::TextDelta(lines)).await;

            // #192: and offer the same rows as a list you can act on. The text
            // above is unchanged, so `archon -p` and anything scraping this
            // output keep exactly what they had — the overlay is additive, and
            // a print-mode run simply drops the event.
            let entries = archon_tools::config_tool::config_entries()
                .into_iter()
                .map(|entry| {
                    (
                        entry.key.to_string(),
                        entry.value,
                        entry.is_bool,
                        entry.read_only,
                    )
                })
                .collect();
            let _ = tui_tx.send_async(TuiEvent::ShowSettings(entries)).await;
        } else if value.is_empty() {
            // Get a single key
            match archon_tools::config_tool::get_config_value(key) {
                Some(val) => {
                    let _ = tui_tx
                        .send_async(TuiEvent::TextDelta(format!("\n{key} = {val}\n")))
                        .await;
                }
                None => {
                    let _ = tui_tx
                        .send_async(TuiEvent::Error(format!("Unknown config key: {key}")))
                        .await;
                }
            }
        } else {
            // Set key=value via the ConfigTool
            use archon_tools::tool::{AgentMode, ToolContext};
            let tool = archon_tools::config_tool::ConfigTool;
            let tool_ctx = ToolContext {
                working_dir: std::env::current_dir().unwrap_or_default(),
                session_id: String::new(),
                mode: AgentMode::Normal,
                extra_dirs: Vec::new(),
                ..Default::default()
            };
            let result = archon_tools::tool::Tool::execute(
                &tool,
                serde_json::json!({ "action": "set", "key": key, "value": value }),
                &tool_ctx,
            )
            .await;
            if result.is_error {
                let _ = tui_tx.send_async(TuiEvent::Error(result.content)).await;
            } else {
                let _ = tui_tx
                    .send_async(TuiEvent::TextDelta(format!("\n{}\n", result.content)))
                    .await;
            }
        }
    })
}

/// Registry entry for `/config`. The body is [`handle_config_command`], run
/// from the interception in `command::slash`; see the module docs.
pub(crate) struct ConfigHandler;

impl ConfigHandler {
    /// The primary spelling, shared by the registry entry in
    /// `registry::default_registry` and by [`command_args`], so the name the
    /// dispatcher knows and the name the interception matches cannot differ.
    pub(crate) const NAME: &'static str = "config";

    /// Construct a fresh `ConfigHandler`. Zero-sized so this is free.
    pub(crate) fn new() -> Self {
        Self
    }
}

impl Default for ConfigHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandHandler for ConfigHandler {
    fn execute(&self, _ctx: &mut CommandContext, _args: &[String]) -> anyhow::Result<()> {
        // Unreachable: `command::slash` intercepts every spelling
        // `command_args` recognises, which is every spelling the registry
        // resolves to this handler. Reaching here means the config command
        // did nothing, and silence about that is what hid the alias defect.
        anyhow::bail!(
            "/config did not run: its body is intercepted before the dispatcher \
             and the dispatcher was reached instead. Nothing was shown or \
             changed. This is a dispatch wiring regression — report it."
        )
    }

    fn description(&self) -> &'static str {
        "Show or update Archon configuration"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &["settings", "prefs"]
    }
}

/// Every spelling the registry resolves to [`ConfigHandler`]: the primary name
/// followed by each published alias. Tests iterate this rather than a
/// hand-written list.
pub(crate) fn spellings() -> Vec<&'static str> {
    std::iter::once(ConfigHandler::NAME)
        .chain(ConfigHandler.aliases().iter().copied())
        .collect()
}

/// The argument text after the command name when `input` is `/config` under
/// any of its [`spellings`]; `None` when `input` is some other command.
pub(crate) fn command_args(input: &str) -> Option<&str> {
    intercept::command_args(input, &spellings())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use crate::command::dispatcher::Dispatcher;
    use crate::command::registry::RegistryBuilder;

    /// Build a minimal `CommandContext` with a freshly-created channel.
    /// /config is a THIN-WRAPPER handler — no snapshot, no effect slot,
    /// no extra context field — so every optional field stays `None`.
    /// Mirrors the `make_ctx` fixtures in compact.rs / clear.rs /
    /// cancel.rs.
    fn make_ctx() -> (CommandContext, archon_tui::event_channel::TuiEventReceiver) {
        // TASK-AGS-POST-6-SHARED-FIXTURES-V2: migrated to CtxBuilder.
        crate::command::test_support::CtxBuilder::new().build()
    }

    #[test]
    fn the_description_and_aliases_the_registry_publishes_are_unchanged() {
        let h = ConfigHandler::new();
        assert_eq!(h.description(), "Show or update Archon configuration");
        assert_eq!(h.aliases(), &["settings", "prefs"]);
        assert_eq!(ConfigHandler::NAME, "config");
    }

    /// Every spelling the registry resolves to `ConfigHandler` must be one the
    /// interception in `command::slash` claims, or that spelling reaches the
    /// unreachable handler and the user gets silence. Driven off `spellings()`
    /// so a new alias is covered the moment it is declared.
    #[test]
    fn every_spelling_the_registry_resolves_here_is_one_the_interception_claims() {
        let mut b = RegistryBuilder::new();
        b.insert_primary(ConfigHandler::NAME, Arc::new(ConfigHandler::new()));
        let registry = b.build();

        for spelling in spellings() {
            assert!(
                registry.get(spelling).is_some(),
                "/{spelling} does not resolve in the registry"
            );
            assert_eq!(
                command_args(&format!("/{spelling}")),
                Some(""),
                "/{spelling} resolves to the config command but the \
                 interception does not recognise it"
            );
            assert_eq!(
                command_args(&format!("/{spelling} model")),
                Some("model"),
                "/{spelling} must carry its arguments through the interception"
            );
        }
    }

    /// Reaching `execute` means the config command did nothing.
    #[test]
    fn reaching_the_handler_body_is_an_error_not_a_silent_success() {
        let mut b = RegistryBuilder::new();
        b.insert_primary(ConfigHandler::NAME, Arc::new(ConfigHandler::new()));
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
                        if message.contains("Nothing was shown or changed")
                )),
                "the user must be told the command did not run, got: {events:?}"
            );
        }
    }
}
