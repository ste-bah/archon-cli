//! What `TerminalCreate` advertises, in the world the session is actually in.
//!
//! The tool described itself once, when the registry was built, and named all
//! four host shells whatever world the session later ran in. Under a docker or
//! ssh backend two of those do not exist there, so the model was told about
//! shells it would be refused for asking about. The refusal in `terminal_world`
//! is correct and stays; what it cost was a turn spent on a call that could
//! never have worked.
//!
//! Everything world-aware here is a projection of `terminal_world::offer`,
//! which is a projection of the same `decide` call `execute` makes. Nothing
//! restates a backend's shell list, so the advertisement cannot drift from the
//! refusal. What the projection does *not* cover is the `cwd` argument: `offer`
//! is asked about the session working directory, and a backend decides on the
//! directory too, so the descriptions below say that rather than imply the
//! shell menu is the whole gate.

use serde_json::json;

use crate::terminal_shell as shells;
use crate::terminal_world::{self as world, Offer};
use crate::tool::ToolContext;

/// The schema for a session whose terminals are host shells.
///
/// This is the world-independent answer, unchanged from before any of this
/// existed.
///
/// **It over-promises on the host, deliberately and for a reason worth naming.**
/// `cmd` is listed on Linux, where `terminal_shell::build` refuses it, and
/// `powershell` is listed without checking PATH — which is not hypothetical:
/// `which` fails to find PowerShell on some Windows shells, and did on the
/// machine this was written on. Neither is corrected here, because these bytes
/// are the prompt-cache prefix every unsandboxed session already has, and
/// changing them is a separate decision with a separate cost: the `cmd` half is
/// free (`cfg!(windows)`, no probe) but changes the Linux surface, and the
/// PowerShell half cannot be fixed without a PATH probe that would make the
/// advertised surface machine-dependent and no longer byte-stable across turns.
/// The sentence about `cmd` below is a mitigation, not a justification.
pub(crate) fn host_schema() -> serde_json::Value {
    schema(
        json!({
            "type": "string",
            "enum": shells::SHELLS,
            "description": format!(
                "Which shell to run (default {}). \"cmd\" is Windows only.",
                shells::default_shell()
            )
        }),
        host_cwd(),
    )
}

/// The schema for the world `ctx` is in, or `None` when that world is the host
/// and [`host_schema`] already describes it.
pub(crate) fn world_schema(ctx: &ToolContext) -> Option<serde_json::Value> {
    match world::offer(ctx, &ctx.working_dir) {
        Offer::Host => None,
        Offer::Shells {
            available,
            default_shell,
            sandboxed,
        } => Some(schema(
            shell_menu(&available, &default_shell, sandboxed),
            if sandboxed { world_cwd() } else { host_cwd() },
        )),
        // The shell argument still carries the reason, for a caller that reads
        // the schema before calling. It is not where the refusal *lives* —
        // [`world_description`] is, because this tool requires no arguments and
        // the call that names none would never reach this string.
        Offer::Refused(reason) => Some(schema(
            json!({
                "type": "string",
                "description": format!("No terminal opens in this session: {reason}")
            }),
            host_cwd(),
        )),
    }
}

/// How the tool describes *itself* in `ctx`'s world, or `None` when the world
/// does not change what it is.
///
/// Only a refusal changes it. A narrowed shell menu is an argument detail and
/// belongs in the schema; a world with no terminal at all is not a detail, and
/// a model choosing tools reads this before it reads any argument.
pub(crate) fn world_description(ctx: &ToolContext, when_available: &str) -> Option<String> {
    let Offer::Refused(reason) = world::offer(ctx, &ctx.working_dir) else {
        return None;
    };
    Some(format!(
        "UNAVAILABLE in this session: {reason} This holds for every call, \
         including one with no arguments, so there is nothing to try. What it \
         does where terminals exist: {when_available}"
    ))
}

fn shell_menu(available: &[String], default_shell: &str, sandboxed: bool) -> serde_json::Value {
    if available.is_empty() {
        // A world that opens a terminal for a bare request but names no shell
        // it will accept. `"enum": []` matches nothing and some providers
        // reject it outright, so the property carries the one usable
        // instruction instead: do not name a shell.
        return json!({
            "type": "string",
            "description": format!(
                "This session's execution world accepts no named shell. Omit \
                 this argument to get {default_shell}."
            )
        });
    }
    let where_they_run = if sandboxed {
        "This session runs its shells in a sandbox, and these are the ones it \
         accepts."
    } else {
        "These are the ones this session accepts; the rest are refused here."
    };
    json!({
        "type": "string",
        "enum": available,
        "description": format!("Which shell to run (default {default_shell}). {where_they_run}")
    })
}

fn host_cwd() -> serde_json::Value {
    json!({
        "type": "string",
        "description": "Directory to start in (default: the session working directory)"
    })
}

/// A sandboxed world decides on the directory as well as the shell, and it is
/// not the same decision: docker refuses a directory outside the workspace
/// mount and ssh in `remote` mode refuses every one but the workspace. A shell
/// being on the menu says nothing about those, so this says it instead of
/// letting the menu imply an all-clear.
fn world_cwd() -> serde_json::Value {
    json!({
        "type": "string",
        "description": "Directory to start in (default: the session working \
                        directory). This session's shells run in a sandbox, \
                        which may refuse a directory outside the session \
                        workspace even when the shell itself is available."
    })
}

/// The one object shape `TerminalCreate` accepts, whatever world describes it.
fn schema(shell: serde_json::Value, cwd: serde_json::Value) -> serde_json::Value {
    json!({
        "type": "object",
        "properties": {
            "shell": shell,
            "cwd": cwd
        },
        "required": []
    })
}

#[cfg(test)]
#[path = "terminal_schema_tests.rs"]
mod tests;
