//! `ToolRegistry::redescribe` — the seam that lets a tool describe itself
//! against the world a session is actually in (#201 review, gap 3).
//!
//! Two properties matter here and nothing else does: a context with no world of
//! its own must hand back the bytes it was given, and a context with one must
//! reach only the tools that declared something for it.

use std::sync::Arc;

use archon_permissions::sandbox::{
    SandboxBackend, SandboxTerminal, SandboxTerminalCommand, SandboxTerminalRequest,
};
use archon_tools::tool::ToolContext;

use super::*;

/// A Linux world with no PowerShell and no cmd, which is what `docker` and
/// `ssh` both are.
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
                return SandboxTerminal::Refused(format!("no {other} in a Linux container"));
            }
        };
        SandboxTerminal::Open(SandboxTerminalCommand {
            program: "docker".into(),
            args: vec!["run".into(), format!("/bin/{shell}")],
            shell: shell.to_string(),
            location: "/workspace in the container".into(),
        })
    }
}

fn ctx(sandbox: Option<Arc<dyn SandboxBackend>>) -> ToolContext {
    ToolContext {
        working_dir: std::env::temp_dir(),
        session_id: "redescribe-tests".into(),
        sandbox,
        ..ToolContext::default()
    }
}

fn registry() -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(archon_tools::terminal_tools::TerminalCreateTool));
    registry.register(Box::new(archon_tools::file_read::ReadTool));
    registry
}

fn shells_offered(definitions: &[serde_json::Value], tool: &str) -> Vec<String> {
    definitions
        .iter()
        .find(|definition| definition["name"] == tool)
        .expect("tool is defined")["input_schema"]["properties"]["shell"]["enum"]
        .as_array()
        .expect("the shell argument is an enum")
        .iter()
        .map(|value| value.as_str().expect("shell names are strings").to_string())
        .collect()
}

/// The no-sandbox pin. Not "equivalent" — identical, because the prompt-cache
/// prefix is bytes and a rebuilt-but-equal definition would still have to be
/// proven equal every turn.
#[test]
fn a_context_with_no_world_hands_back_exactly_what_it_was_given() {
    let registry = registry();
    let definitions = registry.tool_definitions();

    let redescribed = registry.redescribe(&definitions, &ctx(None));

    assert_eq!(definitions, redescribed);
}

#[test]
fn a_sandboxed_context_narrows_the_shells_terminalcreate_offers() {
    let registry = registry();
    let definitions = registry.tool_definitions();
    assert_eq!(
        shells_offered(&definitions, "TerminalCreate"),
        vec!["bash", "sh", "powershell", "cmd"],
        "the host surface still names all four"
    );

    let redescribed = registry.redescribe(&definitions, &ctx(Some(Arc::new(LinuxWorld))));

    assert_eq!(
        shells_offered(&redescribed, "TerminalCreate"),
        vec!["bash", "sh"]
    );
}

/// Re-describing one tool must not disturb the rest of the surface: the pass
/// touches only entries whose tool declared something for this world.
#[test]
fn tools_with_nothing_world_specific_to_say_are_left_alone() {
    let registry = registry();
    let definitions = registry.tool_definitions();

    let redescribed = registry.redescribe(&definitions, &ctx(Some(Arc::new(LinuxWorld))));

    assert_eq!(definitions.len(), redescribed.len());
    let read_before = definitions
        .iter()
        .find(|definition| definition["name"] == "Read");
    let read_after = redescribed
        .iter()
        .find(|definition| definition["name"] == "Read");
    assert_eq!(read_before, read_after);
}

/// The same question put to the shipped backend rather than a stand-in, so a
/// change to `docker::container_shell` shows up here instead of drifting away
/// from a fake that still agrees with the version it was written against.
/// Nothing is executed: `terminal` builds a command line, and the daemon is
/// only needed to run it.
#[test]
fn the_real_docker_backend_narrows_the_surface_to_what_a_container_has() {
    let backend = Arc::new(crate::sandbox::DockerSandboxBackend::new(
        crate::sandbox::DockerConfig {
            enabled: true,
            ..Default::default()
        },
        "rw",
    ));
    let registry = registry();
    let definitions = registry.tool_definitions();

    let redescribed = registry.redescribe(&definitions, &ctx(Some(backend)));

    assert_eq!(
        shells_offered(&redescribed, "TerminalCreate"),
        vec!["bash", "sh"],
        "a Linux container has no PowerShell and no cmd"
    );
}

/// MCP tools and anything else added to the list outside this registry are not
/// this registry's to rewrite, and a definition it cannot recognise must
/// survive the pass rather than be dropped.
#[test]
fn a_definition_for_an_unregistered_tool_passes_through_untouched() {
    let outsider = serde_json::json!({
        "name": "mcp__somewhere__do_thing",
        "description": "not from this registry",
        "input_schema": {"type": "object", "properties": {}},
    });
    let nameless = serde_json::json!({"description": "no name at all"});
    let definitions = vec![outsider, nameless];

    let redescribed = registry().redescribe(&definitions, &ctx(Some(Arc::new(LinuxWorld))));

    assert_eq!(definitions, redescribed);
}
