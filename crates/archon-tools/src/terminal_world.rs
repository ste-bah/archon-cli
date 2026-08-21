//! Which execution world a terminal opens in (#201 Phase 6).
//!
//! `Bash` is routed through the active sandbox backend; terminals were not, so
//! under `sandbox.backend = docker | ssh | openshell` opening a terminal handed
//! the model an unsandboxed host shell while every `Bash` call went through the
//! backend. That is not a gap in coverage, it is a way around the boundary.
//!
//! Everything here follows from one rule: a terminal opens in the same world
//! `Bash` runs in, or it does not open. The backend decides which â€” and the
//! decision is read from `ctx.sandbox` being present rather than from
//! `check()`, because under the default `sandbox.mode = "risky"` `check()` is
//! only consulted for `Bash` and `Shell` and would say nothing at all here.

use std::path::Path;

use archon_permissions::sandbox::{
    SandboxTerminal, SandboxTerminalCommand, SandboxTerminalRequest,
};
use archon_pty::CommandBuilder;

use crate::terminal_shell as shells;
use crate::tool::ToolContext;

/// A terminal that is about to be opened, and where.
#[derive(Debug)]
pub(crate) struct Launch {
    pub(crate) command: CommandBuilder,
    /// The shell that will be running, which is not always the one asked for:
    /// a backend whose world is Linux answers a bare request with bash.
    pub(crate) shell: String,
    /// Where it will be running, phrased in that world's own paths.
    pub(crate) location: String,
    /// Whether it lands inside a backend's world. Recorded so a terminal opened
    /// on the host cannot be written to after a sandbox becomes active.
    pub(crate) sandboxed: bool,
}

/// What the active world will accept from a `TerminalCreate` call.
///
/// Read by the input schema so the model is offered the shells that exist
/// where it is, rather than the four the host build knows about.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Offer {
    /// A shell on this machine, which is what the world-independent schema
    /// already describes. Nothing to say.
    Host,
    /// A backend's world, and the shells it answered for.
    Shells {
        available: Vec<String>,
        default_shell: String,
    },
    /// No shell can be opened at all, and why.
    Refused(String),
}

/// Ask the active world what it would accept, without opening anything.
///
/// Every answer here comes from [`decide`] — the same call [`plan`] makes, with
/// the same arguments. Nothing restates a backend's shell list, so what is
/// advertised cannot drift from what is refused: a backend that stops
/// accepting `sh` stops advertising it in the same commit, and one that starts
/// accepting a fifth shell needs `shells::SHELLS` extended before either side
/// sees it.
pub(crate) fn offer(ctx: &ToolContext, cwd: &Path) -> Offer {
    // The bare request settles two things at once: whether a backend holds the
    // execution world, and what a caller that names no shell actually gets.
    // Taking `shells::default_shell()` for the second would promise PowerShell
    // to a Linux container on a Windows host.
    let (default_shell, on_the_host) = match decide(ctx, None, cwd) {
        SandboxTerminal::Host => (shells::default_shell().to_string(), true),
        SandboxTerminal::Open(command) => (command.shell, false),
        SandboxTerminal::Refused(reason) => return Offer::Refused(reason),
    };
    // A shell stays on the menu unless the world refuses it *by name*. `Host`
    // is not a refusal: it says the backend is not the one deciding this shell,
    // and what remains is the host launcher's own platform limit, which the
    // world-independent description already states.
    let available: Vec<String> = shells::SHELLS
        .iter()
        .filter(|shell| !matches!(decide(ctx, Some(shell), cwd), SandboxTerminal::Refused(_)))
        .map(|shell| (*shell).to_string())
        .collect();
    // Nothing world-specific happened. Asked separately from the bare request
    // because a backend is free to answer `Host` for "any shell" and still
    // refuse a named one, and a schema that stopped at the first answer would
    // then advertise a shell the same backend rejects a moment later.
    if on_the_host && available.len() == shells::SHELLS.len() {
        return Offer::Host;
    }
    Offer::Shells {
        available,
        default_shell,
    }
}

/// Resolve a terminal request to something launchable, or to the reason it
/// cannot be. `Err` is a real answer here, not a failure.
pub(crate) fn plan(ctx: &ToolContext, shell: Option<&str>, cwd: &Path) -> Result<Launch, String> {
    match decide(ctx, shell, cwd) {
        SandboxTerminal::Host => host_launch(shell, cwd),
        SandboxTerminal::Open(command) => Ok(sandbox_launch(command)),
        SandboxTerminal::Refused(reason) => Err(reason),
    }
}

/// Whether a shell on this machine is still what "a terminal" means here.
///
/// False once a backend claims the execution world, which is what makes a
/// terminal opened before `/sandbox on` unusable rather than merely stale.
pub(crate) fn host_terminals_allowed(ctx: &ToolContext) -> bool {
    matches!(decide(ctx, None, &ctx.working_dir), SandboxTerminal::Host)
}

fn decide(ctx: &ToolContext, shell: Option<&str>, cwd: &Path) -> SandboxTerminal {
    let Some(sandbox) = ctx.sandbox.as_ref() else {
        return SandboxTerminal::Host;
    };
    sandbox.terminal(&SandboxTerminalRequest {
        shell: shell.map(str::to_string),
        workspace: ctx.working_dir.clone(),
        cwd: cwd.to_path_buf(),
    })
}

fn host_launch(shell: Option<&str>, cwd: &Path) -> Result<Launch, String> {
    let shell = match shell {
        Some(shell) => shell,
        None => shells::default_shell(),
    };
    Ok(Launch {
        command: shells::build(shell, cwd)?,
        shell: shell.to_string(),
        location: cwd.display().to_string(),
        sandboxed: false,
    })
}

fn sandbox_launch(command: SandboxTerminalCommand) -> Launch {
    let mut built = CommandBuilder::new(&command.program);
    built.args(&command.args);
    // Set on the launcher, not only inside the world: `ssh` forwards the local
    // `TERM` to the remote shell, so leaving it unset there costs the same
    // line-at-a-time degradation the host path already guards against.
    built.env("TERM", "xterm-256color");
    Launch {
        command: built,
        shell: command.shell,
        location: command.location,
        sandboxed: true,
    }
}

#[cfg(test)]
#[path = "terminal_world_tests.rs"]
pub(crate) mod tests;
