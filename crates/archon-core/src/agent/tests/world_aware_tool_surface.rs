// The main agent describes its tool surface against the world it is in.
//
// `AgentConfig::tools` is a snapshot taken at session boot, before the sandbox
// backend exists and long before `/sandbox on` can install one. These tests
// drive the real turn-request path, so removing the re-description from
// `prepare_turn_request` fails them rather than passing quietly.

#[derive(Debug)]
struct LinuxWorldBackend;

impl archon_permissions::SandboxBackend for LinuxWorldBackend {

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

    fn terminal(
        &self,
        request: &archon_permissions::SandboxTerminalRequest,
    ) -> archon_permissions::SandboxTerminal {
        let shell = match request.shell.as_deref() {
            None | Some("bash") => "bash",
            Some("sh") => "sh",
            Some(other) => {
                return archon_permissions::SandboxTerminal::Refused(format!(
                    "no {other} in a Linux container"
                ));
            }
        };
        archon_permissions::SandboxTerminal::Open(archon_permissions::SandboxTerminalCommand {
            program: "docker".into(),
            args: vec!["run".into(), format!("/bin/{shell}")],
            shell: shell.to_string(),
            location: "/workspace in the container".into(),
        })
    }
}

fn terminal_agent(sandbox: Option<Arc<dyn archon_permissions::SandboxBackend>>) -> Agent {
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(archon_tools::terminal_tools::TerminalCreateTool));
    let (tx, _rx) = tokio::sync::mpsc::channel(AGENT_EVENT_CHANNEL_CAPACITY);
    let config = AgentConfig {
        // Exactly how a session boots: the surface is captured from the
        // registry before the backend is decided.
        tools: registry.tool_definitions(),
        sandbox,
        ..AgentConfig::default()
    };
    Agent::new(
        Arc::new(MockLlmProvider),
        registry,
        config,
        tx,
        Arc::new(std::sync::RwLock::new(AgentRegistry::load(
            &std::env::temp_dir(),
        ))),
    )
}

async fn advertised_shells(agent: &mut Agent) -> Vec<String> {
    agent.state.add_user_message("open a shell");
    let prepared = agent
        .prepare_turn_request("open a shell", 0)
        .await
        .expect("request prepared");
    prepared
        .request
        .tools
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
async fn a_session_with_no_backend_is_offered_every_host_shell() {
    let mut agent = terminal_agent(None);

    assert_eq!(
        advertised_shells(&mut agent).await,
        vec!["bash", "sh", "powershell", "cmd"]
    );
}

#[tokio::test]
async fn a_session_in_a_container_is_offered_only_the_shells_it_has() {
    let mut agent = terminal_agent(Some(Arc::new(LinuxWorldBackend)));

    assert_eq!(advertised_shells(&mut agent).await, vec!["bash", "sh"]);
}

/// The surface is rebuilt every turn, so it must not churn every turn: the
/// prompt-cache prefix is compared as bytes, and a tool list that differed
/// between two turns in the same world would discard it.
#[tokio::test]
async fn the_surface_is_byte_stable_while_the_world_is() {
    let mut agent = terminal_agent(Some(Arc::new(LinuxWorldBackend)));

    agent.state.add_user_message("first");
    let first = agent
        .prepare_turn_request("first", 0)
        .await
        .expect("first request");
    agent.state.add_user_message("second");
    let second = agent
        .prepare_turn_request("second", 0)
        .await
        .expect("second request");

    assert_eq!(
        serde_json::to_vec(&*first.request.tools).unwrap(),
        serde_json::to_vec(&*second.request.tools).unwrap()
    );
}
