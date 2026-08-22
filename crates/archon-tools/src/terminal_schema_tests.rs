//! What the model is told `TerminalCreate` accepts, per world.
//!
//! This file holds the stand-in worlds and the helpers that read a schema. The
//! assertions live in [`menus`] and [`refusals`].
//!
//! Each fake answers a *different* combination of the three things
//! `SandboxBackend::terminal` may say, including two no shipping backend
//! produces. That is deliberate: every defect this suite exists to catch is a
//! case where one shell's answer differs from the bare request's, and fakes
//! that all agree with each other prove only that the code agrees with itself.

use std::sync::Arc;

use archon_permissions::sandbox::{
    SandboxBackend, SandboxScope, SandboxScopeSupport, SandboxTerminal, SandboxTerminalCommand,
    SandboxTerminalRequest,
};

use super::*;
use crate::terminal_world::plan;
use crate::terminal_world::tests::{FixedTerminalBackend, door};

/// Every stand-in here holds its world open the way `FixedTerminalBackend` in
/// [`crate::terminal_world::tests`] does. Lifetime is not what these fakes
/// model -- they exist to vary what `terminal` answers -- so they all give the
/// one answer that keeps them consistent with their sibling rather than
/// inventing a lifetime story per fake.
fn stand_in_scope_support(_scope: SandboxScope) -> SandboxScopeSupport {
    SandboxScopeSupport::Held
}

/// A Linux world, answering the way `docker::container_shell` does: bash and
/// sh exist, the two Windows shells do not, and a bare request means bash even
/// when the host would have said PowerShell.
#[derive(Debug)]
struct LinuxWorld;

impl SandboxBackend for LinuxWorld {
    fn scope_support(&self, scope: SandboxScope) -> SandboxScopeSupport {
        stand_in_scope_support(scope)
    }
    fn check(
        &self,
        _tool: &str,
        _capability: archon_permissions::ToolCapability,
        _input: &serde_json::Value,
    ) -> Result<(), String> {
        Ok(())
    }

    fn terminal(&self, request: &SandboxTerminalRequest) -> SandboxTerminal {
        let shell = match request.shell.as_deref() {
            None | Some("bash") => "bash",
            Some("sh") => "sh",
            Some(other) => {
                return SandboxTerminal::Refused(format!(
                    "the container runs Linux, which has no {other}"
                ));
            }
        };
        SandboxTerminal::Open(SandboxTerminalCommand {
            program: "container-door".into(),
            args: vec!["run".into(), "--tty".into(), format!("/bin/{shell}")],
            shell: shell.to_string(),
            location: "/workspace in the ubuntu:24.04 container".into(),
        })
    }
}

/// A world that opens a terminal for a bare request but refuses every shell
/// named explicitly. Nothing ships one; the schema still has to survive it,
/// because `"enum": []` is not a schema a provider will take.
#[derive(Debug)]
struct NamelessWorld;

impl SandboxBackend for NamelessWorld {
    fn scope_support(&self, scope: SandboxScope) -> SandboxScopeSupport {
        stand_in_scope_support(scope)
    }
    fn check(
        &self,
        _tool: &str,
        _capability: archon_permissions::ToolCapability,
        _input: &serde_json::Value,
    ) -> Result<(), String> {
        Ok(())
    }

    fn terminal(&self, request: &SandboxTerminalRequest) -> SandboxTerminal {
        match request.shell {
            None => SandboxTerminal::Open(door()),
            Some(_) => SandboxTerminal::Refused("this world names no shells".into()),
        }
    }
}

/// A world that does not relocate a shell but still bans two of them. Nothing
/// ships one either; it exists because `SandboxBackend::terminal` permits the
/// combination, and a schema that stopped at the bare request's `Host` would
/// go on advertising the two this refuses.
#[derive(Debug)]
struct HostMinusWindowsShells;

impl SandboxBackend for HostMinusWindowsShells {
    fn scope_support(&self, scope: SandboxScope) -> SandboxScopeSupport {
        stand_in_scope_support(scope)
    }
    fn check(
        &self,
        _tool: &str,
        _capability: archon_permissions::ToolCapability,
        _input: &serde_json::Value,
    ) -> Result<(), String> {
        Ok(())
    }

    fn terminal(&self, request: &SandboxTerminalRequest) -> SandboxTerminal {
        match request.shell.as_deref() {
            Some("powershell" | "cmd") => SandboxTerminal::Refused("no Windows shells here".into()),
            _ => SandboxTerminal::Host,
        }
    }
}

/// A world that relocates a bare request into a sandbox but answers `Host` for
/// `sh`. The mirror of the case above and the worse one: advertising `sh` on a
/// sandboxed menu would claim an isolation `plan` does not give it.
#[derive(Debug)]
struct SandboxWithOneHostShell;

impl SandboxBackend for SandboxWithOneHostShell {
    fn scope_support(&self, scope: SandboxScope) -> SandboxScopeSupport {
        stand_in_scope_support(scope)
    }
    fn check(
        &self,
        _tool: &str,
        _capability: archon_permissions::ToolCapability,
        _input: &serde_json::Value,
    ) -> Result<(), String> {
        Ok(())
    }

    fn terminal(&self, request: &SandboxTerminalRequest) -> SandboxTerminal {
        match request.shell.as_deref() {
            Some("sh") => SandboxTerminal::Host,
            Some("powershell" | "cmd") => SandboxTerminal::Refused("Linux container".into()),
            _ => SandboxTerminal::Open(door()),
        }
    }
}

/// The other way round: a world that hosts its shells but relocates `bash`
/// into a sandbox. Advertising `bash` on this menu would be the same defect
/// pointing the other way — the menu's prose says these run here, and that one
/// does not.
///
/// The two Windows shells are refused rather than hosted so that `plan` gives
/// a definite answer for them on every platform; see `host_schema`'s note on
/// what the host launcher refuses at call time.
#[derive(Debug)]
struct HostWithOneSandboxedShell;

impl SandboxBackend for HostWithOneSandboxedShell {
    fn scope_support(&self, scope: SandboxScope) -> SandboxScopeSupport {
        stand_in_scope_support(scope)
    }
    fn check(
        &self,
        _tool: &str,
        _capability: archon_permissions::ToolCapability,
        _input: &serde_json::Value,
    ) -> Result<(), String> {
        Ok(())
    }

    fn terminal(&self, request: &SandboxTerminalRequest) -> SandboxTerminal {
        match request.shell.as_deref() {
            Some("bash") => SandboxTerminal::Open(door()),
            Some("powershell" | "cmd") => SandboxTerminal::Refused("not offered here".into()),
            _ => SandboxTerminal::Host,
        }
    }
}

/// A world whose bare request means `sh`, which is neither platform's host
/// default. Without one, every fake here answers a bare request with the same
/// shell Linux would have picked anyway, and an implementation that ignored
/// the world's answer and returned a literal `bash` would pass the whole suite
/// on Linux and macOS — caught only by Windows CI, which is coverage by
/// accident.
#[derive(Debug)]
struct PosixShWorld;

impl SandboxBackend for PosixShWorld {
    fn scope_support(&self, scope: SandboxScope) -> SandboxScopeSupport {
        stand_in_scope_support(scope)
    }
    fn check(
        &self,
        _tool: &str,
        _capability: archon_permissions::ToolCapability,
        _input: &serde_json::Value,
    ) -> Result<(), String> {
        Ok(())
    }

    fn terminal(&self, request: &SandboxTerminalRequest) -> SandboxTerminal {
        let shell = match request.shell.as_deref() {
            None | Some("sh") => "sh",
            Some("bash") => "bash",
            Some(other) => {
                return SandboxTerminal::Refused(format!("this world has no {other}"));
            }
        };
        SandboxTerminal::Open(SandboxTerminalCommand {
            program: "container-door".into(),
            args: vec!["run".into(), format!("/bin/{shell}")],
            shell: shell.to_string(),
            location: "/workspace in the busybox container".into(),
        })
    }
}

fn ctx(sandbox: Option<Arc<dyn SandboxBackend>>) -> ToolContext {
    ToolContext {
        working_dir: std::env::temp_dir(),
        session_id: "terminal-schema-tests".into(),
        sandbox,
        ..ToolContext::default()
    }
}

fn linux() -> ToolContext {
    ctx(Some(Arc::new(LinuxWorld)))
}

fn shell_property(built: &serde_json::Value) -> &serde_json::Value {
    &built["properties"]["shell"]
}

fn advertised(built: &serde_json::Value) -> Vec<String> {
    shell_property(built)["enum"]
        .as_array()
        .map(|values| {
            values
                .iter()
                .map(|value| value.as_str().expect("shell names are strings").to_string())
                .collect()
        })
        .unwrap_or_default()
}

fn described(built: &serde_json::Value) -> &str {
    shell_property(built)["description"]
        .as_str()
        .expect("the shell argument is described")
}

// Explicit paths: this module is itself `#[path]`-loaded, so a bare `mod`
// would be looked for under the module's name rather than beside this file.
#[path = "terminal_schema_tests/menus.rs"]
mod menus;
#[path = "terminal_schema_tests/refusals.rs"]
mod refusals;
