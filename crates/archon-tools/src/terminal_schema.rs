//! What `TerminalCreate` advertises, in the world the session is actually in.
//!
//! The tool described itself once, when the registry was built, and named all
//! four host shells whatever world the session later ran in. Under a docker or
//! ssh backend two of those do not exist there, so the model was told about
//! shells it would be refused for asking about. The refusal in `terminal_world`
//! is correct and stays; what it cost was a turn spent on a call that could
//! never have worked.
//!
//! Both schemas below are built from one skeleton and one shell list, and the
//! world-aware one is a projection of `terminal_world::offer` — which is itself
//! a projection of the call `execute` makes. There is no second opinion here
//! for the call-time answer to disagree with.

use serde_json::json;

use crate::terminal_shell as shells;
use crate::terminal_world::{self as world, Offer};
use crate::tool::ToolContext;

/// The schema for a session whose terminals are host shells.
///
/// This is the world-independent answer, and it is what a session with no
/// sandbox backend has always been given.
pub(crate) fn host_schema() -> serde_json::Value {
    schema(json!({
        "type": "string",
        "enum": shells::SHELLS,
        "description": format!(
            "Which shell to run (default {}). \"cmd\" is Windows only.",
            shells::default_shell()
        )
    }))
}

/// The schema for the world `ctx` is in, or `None` when that world is the host
/// and [`host_schema`] already describes it.
pub(crate) fn world_schema(ctx: &ToolContext) -> Option<serde_json::Value> {
    match world::offer(ctx, &ctx.working_dir) {
        Offer::Host => None,
        Offer::Shells {
            available,
            default_shell,
        } => Some(schema(sandboxed_shell(&available, &default_shell))),
        Offer::Refused(reason) => Some(schema(json!({
            "type": "string",
            "description": format!(
                "No terminal can be opened in this session, whatever is asked \
                 for: {reason}"
            )
        }))),
    }
}

fn sandboxed_shell(available: &[String], default_shell: &str) -> serde_json::Value {
    if available.is_empty() {
        // A backend that opens a terminal for a bare request but names no shell
        // it will accept explicitly. `"enum": []` matches nothing and some
        // providers reject it outright, so the property carries the one usable
        // instruction instead: do not name a shell.
        return json!({
            "type": "string",
            "description": format!(
                "This session's execution world accepts no named shell. Omit \
                 this argument to get {default_shell}."
            )
        });
    }
    json!({
        "type": "string",
        "enum": available,
        "description": format!(
            "Which shell to run (default {default_shell}). This session runs \
             its shells in a sandbox, and these are the ones it accepts."
        )
    })
}

/// The one object shape `TerminalCreate` accepts, whatever world describes it.
fn schema(shell: serde_json::Value) -> serde_json::Value {
    json!({
        "type": "object",
        "properties": {
            "shell": shell,
            "cwd": {
                "type": "string",
                "description": "Directory to start in (default: the session working directory)"
            }
        },
        "required": []
    })
}

#[cfg(test)]
#[path = "terminal_schema_tests.rs"]
mod tests;
