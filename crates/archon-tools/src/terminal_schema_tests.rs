//! What the model is told `TerminalCreate` accepts, per world.
//!
//! The assertion that carries the whole file is
//! `every_advertised_shell_opens_and_every_omitted_one_is_refused`: it runs the
//! advertised list back through `terminal_world::plan` — the call `execute`
//! makes — so a schema that promised something the world would refuse fails
//! here rather than in a wasted turn.

use std::sync::Arc;

use archon_permissions::sandbox::{
    SandboxBackend, SandboxTerminal, SandboxTerminalCommand, SandboxTerminalRequest,
};

use super::*;
use crate::terminal_world::plan;
use crate::terminal_world::tests::{FixedTerminalBackend, door};

/// A Linux world, answering the way `docker::container_shell` does: bash and
/// sh exist, the two Windows shells do not, and a bare request means bash even
/// when the host would have said PowerShell.
#[derive(Debug)]
struct LinuxWorld;

impl SandboxBackend for LinuxWorld {
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

#[test]
fn the_host_schema_offers_every_shell_and_names_the_platform_default() {
    let built = host_schema();

    assert_eq!(advertised(&built), shells::SHELLS.to_vec());
    assert!(
        described(&built).contains(shells::default_shell()),
        "{}",
        described(&built)
    );
}

/// The pin on the no-sandbox case. A context with no world of its own declares
/// nothing, so the surface an unsandboxed session sees is not rebuilt — it is
/// untouched.
#[test]
fn a_session_with_no_backend_is_described_exactly_as_before() {
    assert_eq!(world_schema(&ctx(None)), None);
    assert_eq!(
        world_schema(&ctx(Some(FixedTerminalBackend::host()))),
        None,
        "a policy-only backend runs host shells, so it has nothing to re-describe"
    );
}

#[test]
fn a_linux_world_advertises_only_the_shells_it_has() {
    let built = world_schema(&linux()).expect("a container is not a host");

    assert_eq!(advertised(&built), vec!["bash", "sh"]);
    assert!(
        !described(&built).contains("powershell") && !described(&built).contains("cmd"),
        "{}",
        described(&built)
    );
}

/// The default differs per world: the host default on Windows is PowerShell,
/// and promising that to a Linux container would refuse every terminal the
/// model opened without naming a shell.
#[test]
fn the_advertised_default_is_the_one_a_bare_request_actually_gets() {
    let ctx = linux();
    let built = world_schema(&ctx).expect("a container is not a host");

    let opened = plan(&ctx, None, &ctx.working_dir).expect("a bare request opens");
    assert_eq!(opened.shell, "bash");
    assert!(
        described(&built).contains(&format!("default {}", opened.shell)),
        "schema says {:?}, the call opens {}",
        described(&built),
        opened.shell
    );
}

/// The whole point, stated as a round trip. Everything the schema offers must
/// open, and everything it leaves out must be refused — checked against `plan`,
/// which is the function `TerminalCreateTool::execute` calls.
#[test]
fn every_advertised_shell_opens_and_every_omitted_one_is_refused() {
    let ctx = linux();
    let built = world_schema(&ctx).expect("a container is not a host");
    let offered = advertised(&built);

    assert!(
        !offered.is_empty(),
        "a world that opens terminals must offer something"
    );
    for shell in shells::SHELLS {
        let is_offered = offered.iter().any(|name| name == shell);
        let outcome = plan(&ctx, Some(shell), &ctx.working_dir);
        assert_eq!(
            is_offered,
            outcome.is_ok(),
            "{shell}: advertised={is_offered}, plan refused with {:?}",
            outcome.err()
        );
    }
}

#[test]
fn a_world_that_cannot_host_a_shell_says_so_instead_of_offering_four() {
    let built = world_schema(&ctx(Some(FixedTerminalBackend::refusing(
        "openshell sandbox: throwaway sandbox per command",
    ))))
    .expect("a refusal is something to say");

    assert!(advertised(&built).is_empty());
    assert!(
        described(&built).contains("throwaway sandbox per command"),
        "{}",
        described(&built)
    );
}

#[test]
fn a_world_that_accepts_no_named_shell_offers_no_enum_at_all() {
    let ctx = ctx(Some(Arc::new(NamelessWorld)));
    let built = world_schema(&ctx).expect("it is not a host");

    assert!(
        shell_property(&built).get("enum").is_none(),
        "an empty enum matches nothing and providers reject it"
    );
    assert!(described(&built).contains("Omit"), "{}", described(&built));
    assert!(plan(&ctx, Some("bash"), &ctx.working_dir).is_err());
    assert!(plan(&ctx, None, &ctx.working_dir).is_ok());
}

/// Every world describes the same argument object, so the only thing that
/// changes between sessions is the shell menu.
#[test]
fn the_argument_shape_is_the_same_in_every_world() {
    let host = host_schema();
    let sandboxed = world_schema(&linux()).expect("a container is not a host");

    for built in [&host, &sandboxed] {
        assert_eq!(built["type"], "object");
        assert_eq!(built["required"], serde_json::json!([]));
        assert_eq!(built["properties"]["cwd"], host["properties"]["cwd"]);
        assert_eq!(shell_property(built)["type"], "string");
    }
}

/// A host shell for the bare request does not license the whole host menu. The
/// backend is asked about each shell separately, because it is allowed to
/// answer differently — and here it does.
#[test]
fn a_world_that_hosts_shells_but_bans_two_advertises_only_the_rest() {
    let ctx = ctx(Some(Arc::new(HostMinusWindowsShells)));
    let built =
        world_schema(&ctx).expect("two shells were taken away, so there is something to say");

    assert_eq!(advertised(&built), vec!["bash", "sh"]);
    for shell in ["powershell", "cmd"] {
        assert!(
            plan(&ctx, Some(shell), &ctx.working_dir).is_err(),
            "{shell} is advertised nowhere and must open nowhere"
        );
    }
}

/// A world can only narrow the menu, never add to it: `offer` filters
/// `shells::SHELLS`, the same list the host launcher builds from, so a backend
/// cannot advertise a shell nothing here knows how to name.
#[test]
fn no_world_can_advertise_a_shell_the_launcher_does_not_know() {
    for backend in [
        Arc::new(LinuxWorld) as Arc<dyn SandboxBackend>,
        FixedTerminalBackend::opening(door()),
    ] {
        let Some(built) = world_schema(&ctx(Some(backend))) else {
            continue;
        };
        for shell in advertised(&built) {
            assert!(
                shells::SHELLS.contains(&shell.as_str()),
                "{shell} is not a shell this build can launch"
            );
        }
    }
}

#[test]
fn the_tool_itself_answers_with_the_world_it_is_asked_about() {
    use crate::terminal_tools::TerminalCreateTool;
    use crate::tool::Tool;

    assert_eq!(TerminalCreateTool.input_schema(), host_schema());
    assert_eq!(TerminalCreateTool.input_schema_for(&ctx(None)), None);
    assert_eq!(
        TerminalCreateTool.input_schema_for(&linux()),
        world_schema(&linux())
    );
}
