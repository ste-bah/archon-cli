use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandHandler};

/// `/providers` handler — emits provider registry, status, capabilities, or doctor text.
pub(crate) struct ProvidersHandler;

impl CommandHandler for ProvidersHandler {
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> anyhow::Result<()> {
        let rendered = match args.first().map(String::as_str) {
            Some("capabilities") | Some("capability") | Some("caps") => {
                archon_llm::providers::render_capability_table()
            }
            Some("doctor") | Some("diagnose") => {
                crate::command::providers::render_provider_doctor(args_contains(args, "--live"))
            }
            Some("status") => {
                let provider = arg_value(args, "--provider");
                crate::command::providers_status::render_provider_status_with_config_and_live(
                    provider,
                    &archon_core::config::ArchonConfig::default(),
                    args_contains(args, "--live"),
                )
            }
            Some("list") | None => crate::command::providers::render_provider_registry(),
            Some(other) => format!(
                "Unknown /providers subcommand `{other}`.\nUsage: /providers [list|status [--provider <id>] [--live]|capabilities|doctor [--live]]\n"
            ),
        };
        ctx.emit(TuiEvent::TextDelta(rendered));
        Ok(())
    }

    fn description(&self) -> &str {
        "List registered LLM providers and capability support"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &[]
    }
}

fn args_contains(args: &[String], needle: &str) -> bool {
    args.iter().any(|arg| arg == needle)
}

fn arg_value<'a>(args: &'a [String], needle: &str) -> Option<&'a str> {
    args.windows(2)
        .find(|window| window[0] == needle)
        .map(|window| window[1].as_str())
}
