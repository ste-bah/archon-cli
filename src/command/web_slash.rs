//! `/web` — start the dashboard inside this TUI process.
//!
//! The `archon web` subcommand cannot attach to a running session by
//! construction: it is a different process. This is the entry point that
//! observes the session you are actually in. See `web_attach` for why that
//! distinction is not cosmetic.

use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandHandler};
use crate::command::web_attach::{self, AttachOptions};

/// Default port for the attached dashboard, matching `WebConfig::default`.
const DEFAULT_PORT: u16 = 8421;

pub(crate) struct WebHandler;

impl CommandHandler for WebHandler {
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> anyhow::Result<()> {
        let rendered = match args.first().map(String::as_str) {
            Some("stop") => match web_attach::request_stop() {
                Some(url) => format!("Web dashboard at {url} is shutting down.\n"),
                None => "No attached web dashboard is running.\n".to_string(),
            },
            Some("status") => match web_attach::running_url() {
                Some(url) => format!("Web dashboard attached to this session: {url}\n"),
                None => "No attached web dashboard is running. Start one with /web.\n".to_string(),
            },
            Some(other) if other.parse::<u16>().is_ok() => {
                start(ctx, other.parse().unwrap_or(DEFAULT_PORT))
            }
            Some(other) => format!(
                "Unknown /web argument `{other}`.\nUsage: /web [port] | /web stop | /web status\n"
            ),
            None => start(ctx, DEFAULT_PORT),
        };
        ctx.emit(TuiEvent::TextDelta(rendered));
        Ok(())
    }

    fn description(&self) -> &str {
        "Start the web dashboard attached to this session"
    }
}

fn start(ctx: &CommandContext, port: u16) -> String {
    let Some(working_dir) = ctx.working_dir.clone() else {
        return "Web dashboard unavailable: session working directory is not wired.\n".to_string();
    };
    let options = AttachOptions {
        port,
        working_dir,
        // The session's own memory handle, so the dashboard does not open a
        // second connection to the same database.
        memory: ctx.memory.clone(),
    };
    match web_attach::start(options) {
        Ok(url) => format!(
            "Web dashboard attached to this session: {url}\n\
             It reports the agents THIS session spawned. Chat is served by the TUI, \
             so the dashboard's chat tab is hidden in this mode.\n\
             Stop it with /web stop; it also stops when the session ends.\n"
        ),
        Err(error) => format!("Web dashboard failed to start: {error:#}\n"),
    }
}
