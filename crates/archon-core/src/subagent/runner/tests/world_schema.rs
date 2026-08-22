//! A subagent is offered the shells that exist where it will run.
//!
//! The executor picks a subagent's toolset from names, before it has decided
//! where the child runs; `SubagentRunner::new` is the first place that holds
//! the definitions and the child's own context at once, and is where the
//! surface is described. Dropping the re-description there fails these.

use std::sync::Arc;

use archon_permissions::sandbox::{
    SandboxBackend, SandboxTerminal, SandboxTerminalCommand, SandboxTerminalRequest,
};
use archon_tools::tool::ToolContext;

use super::*;

#[derive(Debug)]
struct LinuxWorldBackend;

impl SandboxBackend for LinuxWorldBackend {
    // The world this fake stands in for is a container held open across
    // commands, so `Held` is what it would really answer. Lifetime is not what
    // these tests vary; the method is required so no backend can leave the
    // question unanswered.
    fn scope_support(
        &self,
        _scope: archon_permissions::SandboxScope,
    ) -> archon_permissions::SandboxScopeSupport {
        archon_permissions::SandboxScopeSupport::Held
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

fn runner_shells(sandbox: Option<Arc<dyn SandboxBackend>>) -> Vec<String> {
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(archon_tools::terminal_tools::TerminalCreateTool));
    let registry = Arc::new(registry);
    let tool_defs = registry.tool_definitions();
    let ctx = ToolContext {
        working_dir: std::env::temp_dir(),
        session_id: "subagent-world-schema".into(),
        sandbox,
        ..Default::default()
    };
    let runner = SubagentRunner::new(
        Arc::new(MockProvider::new(vec![])),
        "You are a test subagent.".into(),
        tool_defs,
        registry,
        ctx,
        "mock-model".into(),
        1,
        300,
        Arc::new(AgentConfig::default()),
        Arc::new(IdentityProvider::new(
            IdentityMode::Clean,
            "test".into(),
            String::new(),
            String::new(),
        )),
    );

    runner
        .tool_definitions
        .iter()
        .find(|definition| definition["name"] == "TerminalCreate")
        .expect("TerminalCreate is on the surface")["input_schema"]["properties"]["shell"]["enum"]
        .as_array()
        .expect("the shell argument is an enum")
        .iter()
        .map(|value| value.as_str().expect("shell names are strings").to_string())
        .collect()
}

#[tokio::test]
async fn a_subagent_with_no_backend_keeps_the_host_shells() {
    assert_eq!(runner_shells(None), vec!["bash", "sh", "powershell", "cmd"]);
}

#[tokio::test]
async fn a_subagent_in_a_container_is_offered_only_the_shells_it_has() {
    assert_eq!(
        runner_shells(Some(Arc::new(LinuxWorldBackend))),
        vec!["bash", "sh"]
    );
}
