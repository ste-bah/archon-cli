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

/// A world that isolates by refusing rather than relocating — what
/// `/sandbox on` with no backend configured is, and what openshell is.
#[derive(Debug)]
struct NoTerminalsWorld;

impl SandboxBackend for NoTerminalsWorld {
    fn check(
        &self,
        _tool: &str,
        _capability: archon_permissions::ToolCapability,
        _input: &serde_json::Value,
    ) -> Result<(), String> {
        Ok(())
    }

    fn terminal(&self, _request: &SandboxTerminalRequest) -> SandboxTerminal {
        SandboxTerminal::Refused("nothing this session can open is a shell".into())
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

fn description_of(definitions: &[serde_json::Value], tool: &str) -> String {
    definitions
        .iter()
        .find(|definition| definition["name"] == tool)
        .expect("tool is defined")["description"]
        .as_str()
        .expect("a description")
        .to_string()
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

    let redescribed = registry.redescribe(definitions.clone(), &ctx(None));

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

    let redescribed = registry.redescribe(definitions.clone(), &ctx(Some(Arc::new(LinuxWorld))));

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

    let redescribed = registry.redescribe(definitions.clone(), &ctx(Some(Arc::new(LinuxWorld))));

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

    let redescribed = registry.redescribe(definitions.clone(), &ctx(Some(backend)));

    assert_eq!(
        shells_offered(&redescribed, "TerminalCreate"),
        vec!["bash", "sh"],
        "a Linux container has no PowerShell and no cmd"
    );
}

/// A world with no terminal in it has to reach the tool's own description, not
/// just an argument's. `TerminalCreate` requires nothing, so `TerminalCreate {}`
/// is the call a model makes, and it never reads an argument description on the
/// way there.
#[test]
fn a_world_that_refuses_a_tool_outright_rewrites_its_description() {
    let registry = registry();
    let definitions = registry.tool_definitions();
    let refusing = ctx(Some(Arc::new(NoTerminalsWorld)));

    let redescribed = registry.redescribe(definitions.clone(), &refusing);

    let described = redescribed
        .iter()
        .find(|definition| definition["name"] == "TerminalCreate")
        .expect("still on the surface")["description"]
        .as_str()
        .expect("a description");
    assert!(described.contains("UNAVAILABLE"), "{described}");
    assert!(
        described.contains("nothing this session can open is a shell"),
        "the world's own reason has to survive into the description: {described}"
    );

    // Nothing else moved, and a world that merely narrows the menu leaves the
    // description alone.
    let read_before = definitions.iter().find(|d| d["name"] == "Read");
    assert_eq!(
        read_before,
        redescribed.iter().find(|d| d["name"] == "Read")
    );
    let narrowed = registry.redescribe(definitions.clone(), &ctx(Some(Arc::new(LinuxWorld))));
    assert_eq!(
        description_of(&narrowed, "TerminalCreate"),
        description_of(&definitions, "TerminalCreate"),
        "a narrowed shell menu is an argument detail, not a change of what the tool is"
    );
}

/// ToolSearch is a second copy of the tool surface, and the model reaches for
/// it precisely when it doubts a capability is there — `docs/reference/tools.md`
/// calls it "the live tool surface", and at least one shipped agent is told
/// that concluding a tool is absent *without* searching is an anti-pattern. Two
/// copies that agree on something false cost a turn; two that disagree are
/// worse than either, so this pins them equal in one context.
#[tokio::test]
async fn toolsearch_answers_the_same_as_the_live_surface() {
    use archon_tools::tool::Tool;

    let registry = registry();
    let search = archon_tools::toolsearch::ToolSearchTool::new(registry.tool_handles());
    let ctx = ctx(Some(Arc::new(LinuxWorld)));

    let live = registry.redescribe(registry.tool_definitions(), &ctx);
    let searched = search
        .execute(serde_json::json!({"query": "select:TerminalCreate"}), &ctx)
        .await;
    let searched: Vec<serde_json::Value> =
        serde_json::from_str(&searched.content).expect("valid json");

    let live_entry = live
        .iter()
        .find(|definition| definition["name"] == "TerminalCreate")
        .expect("on the live surface");
    assert_eq!(
        shells_offered(&live, "TerminalCreate"),
        vec!["bash", "sh"],
        "the live surface must be the narrowed one, or this proves nothing"
    );
    assert_eq!(&searched[0], live_entry);
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

    let redescribed = registry().redescribe(definitions.clone(), &ctx(Some(Arc::new(LinuxWorld))));

    assert_eq!(definitions, redescribed);
}
