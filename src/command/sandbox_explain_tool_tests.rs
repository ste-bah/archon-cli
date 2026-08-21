//! The `--tool` half of `sandbox explain`: what it reports about one tool.
//!
//! A sibling of `sandbox_cli_tests` rather than more of it, because the two ask
//! different questions - that file checks the configuration sections the
//! command prints, this one checks the decision it reaches about a tool, which
//! is where every defect in this area has been.

use super::*;

#[test]
fn sandbox_explain_can_show_tool_routing_without_execution() {
    let body = render_explain(
        &openshell_config("risky"),
        None,
        Some("Bash"),
        Some("cargo test"),
    )
    .unwrap();

    assert!(body.contains("Tool: Bash"));
    assert_eq!(decision(&body), "route_to_sandbox_when_backend_ready");
    assert!(body.contains("Command preview: cargo test"));
}

fn docker_config(mode: &str) -> archon_core::sandbox::SandboxConfig {
    archon_core::sandbox::SandboxConfig {
        backend: "docker".into(),
        mode: mode.into(),
        docker: archon_core::sandbox::DockerConfig {
            enabled: true,
            ..archon_core::sandbox::DockerConfig::default()
        },
        ..archon_core::sandbox::SandboxConfig::default()
    }
}

fn openshell_config(mode: &str) -> archon_core::sandbox::SandboxConfig {
    archon_core::sandbox::SandboxConfig {
        backend: "openshell".into(),
        mode: mode.into(),
        openshell: archon_core::sandbox::OpenShellConfig {
            enabled: true,
            ..archon_core::sandbox::OpenShellConfig::default()
        },
        ..archon_core::sandbox::SandboxConfig::default()
    }
}

/// The decision, exactly. `contains("Decision: route_to_sandbox")` also matches
/// `route_to_sandbox_when_backend_ready`, which is a weaker claim — asserting
/// by substring would let the two be swapped for each other unnoticed.
fn decision(body: &str) -> &str {
    body.lines()
        .find_map(|line| line.strip_prefix("Decision: "))
        .expect("every tool explanation states a decision")
}

fn reason(body: &str) -> &str {
    body.lines()
        .find_map(|line| line.strip_prefix("Reason: "))
        .expect("every tool explanation states a reason")
}

/// The class comes off the tool. Asserting the label rather than the decision
/// is what pins that down: a hand-written table here could still produce
/// `route_to_sandbox` for `Write` while knowing nothing about what `Write`
/// declares.
#[test]
fn sandbox_explain_reports_the_class_the_tool_declares() {
    let body = render_explain(&docker_config("all"), None, Some("Write"), None).unwrap();

    assert!(
        body.contains("Capability: world-bound/file-write"),
        "explain did not resolve Write to its declared class: {body}"
    );
}

/// A name nothing declares has no decision. A plausible default here would be
/// this command confidently describing the routing of a tool that does not
/// exist.
#[test]
fn sandbox_explain_says_so_when_no_tool_declares_the_name() {
    let body = render_explain(
        &docker_config("all"),
        None,
        Some("NoSuchToolExistsHere"),
        None,
    )
    .unwrap();

    assert_eq!(decision(&body), "unknown_tool");
    assert!(
        !body.contains("Capability: "),
        "a name nothing declares cannot have a class: {body}"
    );
}

/// The registry this command builds is narrower than a session's, so "unknown"
/// has to say which conditions it did not reproduce. Naming only MCP was wrong
/// for the LEANN tools, which a session registers and this command cannot.
#[test]
fn sandbox_explain_says_why_its_registry_is_narrower_than_a_sessions() {
    let body = render_explain(
        &docker_config("all"),
        None,
        Some("NoSuchToolExistsHere"),
        None,
    )
    .unwrap();

    assert!(reason(&body).contains("LEANN"), "{body}");
    assert!(reason(&body).contains("MCP"), "{body}");
    assert!(
        reason(&body).contains("a session would have them"),
        "the reason has to say a session is not this: {body}"
    );
}

/// `Shell` is named by the mode's shell list and declared by no tool, so two
/// sources disagree about whether it exists. Reporting only the registry's
/// half would hide the half that routes.
#[test]
fn sandbox_explain_notes_a_shell_name_the_registry_does_not_have() {
    let shell = render_explain(&docker_config("risky"), None, Some("Shell"), None).unwrap();
    let invented = render_explain(&docker_config("risky"), None, Some("Nonsense"), None).unwrap();

    assert_eq!(decision(&shell), "unknown_tool");
    assert!(
        shell.contains("Note: sandbox.mode names Shell as shell execution"),
        "explain hid that the mode would route this name: {shell}"
    );
    assert!(
        !invented.contains("Note: sandbox.mode names"),
        "a name the mode does not list must not get the note: {invented}"
    );
}

/// Not "explain has a rule about lsp" — explain asked the gate, and the gate's
/// own words come back. If the arm in `check_capability` changes, this changes
/// with it.
#[test]
fn sandbox_explain_reports_the_backends_own_refusal_for_a_host_handle() {
    let body = render_explain(&docker_config("all"), None, Some("lsp"), None).unwrap();

    assert!(
        body.contains("Capability: world-bound/host-handle"),
        "{body}"
    );
    assert_eq!(decision(&body), "blocked_by_backend");
    assert!(
        reason(&body).contains("host handle the sandbox cannot redirect"),
        "the denial is not the gate's own message: {body}"
    );
}

#[test]
fn sandbox_explain_reports_the_backends_own_refusal_for_egress() {
    let body = render_explain(&docker_config("all"), None, Some("WebFetch"), None).unwrap();

    assert!(body.contains("Capability: egress"), "{body}");
    assert_eq!(decision(&body), "blocked_by_backend");
    assert!(reason(&body).contains("leaves the machine"), "{body}");
}

/// The reason is the gate's, not one composed here. Composing one produced
/// "the backend has a seam for this work" over `HostLocal` — which is allowed
/// precisely because nothing is carried anywhere — and over `ControlPlane`,
/// where there is no seam at all.
#[test]
fn sandbox_explain_relays_the_gates_reason_instead_of_composing_one() {
    for (tool, capability) in [
        ("TodoWrite", archon_permissions::ToolCapability::HostLocal),
        ("Agent", archon_permissions::ToolCapability::ControlPlane),
        ("Bash", archon_permissions::ToolCapability::EXECUTION),
    ] {
        let body = render_explain(&docker_config("all"), None, Some(tool), None).unwrap();
        let from_the_gate = archon_core::sandbox::check_capability("docker", tool, capability)
            .expect("allowed")
            .reason();

        assert_eq!(
            reason(&body),
            from_the_gate,
            "{tool}'s reason was written here rather than relayed"
        );
    }

    let host_local = render_explain(&docker_config("all"), None, Some("TodoWrite"), None).unwrap();
    let control_plane = render_explain(&docker_config("all"), None, Some("Agent"), None).unwrap();

    assert!(
        !reason(&host_local).contains("seam"),
        "host-local state is not carried by a seam: {host_local}"
    );
    assert!(
        !reason(&control_plane).contains("seam"),
        "spawning is allowed because a child comes back through the gate, not by a seam: \
         {control_plane}"
    );
}

/// The gate is asked about the class, so an allow is true of a backend that is
/// ready. With `docker.enabled = false` the real `check` returns "docker
/// sandbox backend is disabled", and a bare `route_to_sandbox` would have read
/// as an unconditional promise.
#[test]
fn sandbox_explain_marks_an_allow_as_conditional_on_readiness() {
    let body = render_explain(&docker_config("all"), None, Some("Write"), None).unwrap();

    assert_eq!(decision(&body), "route_to_sandbox_when_backend_ready");
    assert!(body.contains("Readiness: "), "{body}");
}

/// A denial is not conditional — a host handle is a host handle on a running
/// container too — so it must not carry the readiness hedge.
#[test]
fn sandbox_explain_does_not_hedge_a_denial() {
    let body = render_explain(&docker_config("all"), None, Some("lsp"), None).unwrap();

    assert!(
        !body.contains("Readiness: "),
        "a denial that holds in every state was hedged: {body}"
    );
}

/// The regression this file existed to cause: openshell has no session for a
/// terminal to live in and refuses by name in `terminal()`, and reading only
/// the capability gate reported it as routing a shell into its world. The gate
/// says outright that the answer is not its to give.
#[test]
fn sandbox_explain_does_not_claim_openshell_relocates_a_terminal() {
    for mode in ["risky", "shell", "all"] {
        let body =
            render_explain(&openshell_config(mode), None, Some("TerminalCreate"), None).unwrap();

        assert_eq!(
            decision(&body),
            "refused_by_backend_terminal",
            "{mode} mode reported a terminal openshell cannot open: {body}"
        );
        assert!(
            reason(&body).contains("no session for a terminal to live in"),
            "the refusal is not the backend's own: {body}"
        );
    }
}

/// The same class, the same mode, opposite answers — which is why the answer
/// cannot come from the class. Docker attaches a TTY to a container and says
/// where the shell comes up; openshell has nowhere to put one.
#[test]
fn sandbox_explain_asks_each_backend_about_its_own_terminal() {
    let config = docker_config("all");
    let docker = render_explain(&config, None, Some("TerminalCreate"), None).unwrap();
    let openshell =
        render_explain(&openshell_config("all"), None, Some("TerminalCreate"), None).unwrap();

    assert_eq!(decision(&docker), "route_to_sandbox");
    assert!(
        reason(&docker).contains("/workspace in the")
            && reason(&docker).contains(config.docker.image.as_str()),
        "docker's answer should name where the shell comes up, in its own configured image: \
         {docker}"
    );
    assert_eq!(decision(&openshell), "refused_by_backend_terminal");
    assert!(
        docker.contains("Consulted: `SandboxBackend::terminal`"),
        "an answer that includes readiness has to say it consulted the seam: {docker}"
    );
}

/// `mode` scopes `check`, never `terminal`, so under `risky` the permission is
/// the preflight's while the shell is still the backend's. Saying only the
/// first would imply the terminal opens on the host.
#[test]
fn sandbox_explain_separates_terminal_permission_from_terminal_placement() {
    let body = render_explain(&docker_config("risky"), None, Some("TerminalCreate"), None).unwrap();

    assert_eq!(decision(&body), "route_to_sandbox");
    assert!(
        reason(&body).contains("leaves this tool's permission to the normal preflight"),
        "{body}"
    );
    assert!(
        reason(&body).contains("the shell itself is the backend's under every mode"),
        "{body}"
    );
}

/// The subtlety the whole command turns on: under the default mode the backend
/// is never asked about `Write`, so it must not be reported as routed. The
/// wrapper answers `Ok(())` for this call, and an explanation that read only
/// the `Ok` would say the sandbox allowed something it never saw.
#[test]
fn sandbox_explain_risky_mode_does_not_claim_the_backend_decided() {
    let body = render_explain(&docker_config("risky"), None, Some("Write"), None).unwrap();

    assert_eq!(decision(&body), "permission_preflight_only");
    assert!(reason(&body).contains("sandbox.mode = risky"), "{body}");
}

/// The clause about `ToolContext::fs` and `terminal()` is true of the mode
/// whatever the tool is, and irrelevant to a tool with no world to land in.
/// Told about `TodoWrite` it describes two seams the call never touches.
#[test]
fn sandbox_explain_does_not_describe_seams_to_a_tool_that_has_none() {
    let host_local =
        render_explain(&docker_config("risky"), None, Some("TodoWrite"), None).unwrap();
    let world_bound = render_explain(&docker_config("risky"), None, Some("Write"), None).unwrap();

    assert_eq!(decision(&host_local), "permission_preflight_only");
    assert!(
        !reason(&host_local).contains("`ToolContext::fs`"),
        "host-local state does not reach the world through the filesystem seam: {host_local}"
    );
    assert!(
        reason(&world_bound).contains("`ToolContext::fs`"),
        "a tool whose writes land in a world still needs to be told where: {world_bound}"
    );
}

/// `shell` mode defers the same tools as `risky`; only `all` widens it. A
/// decision keyed on the backend alone would report the same thing for both,
/// and be wrong for one of them.
#[test]
fn sandbox_explain_follows_the_mode_rather_than_the_backend() {
    let risky = render_explain(&docker_config("risky"), None, Some("Edit"), None).unwrap();
    let all = render_explain(&docker_config("all"), None, Some("Edit"), None).unwrap();

    assert_eq!(decision(&risky), "permission_preflight_only");
    assert_eq!(decision(&all), "route_to_sandbox_when_backend_ready");
}

/// PowerShell is refused by the mode wrapper itself, before any backend — a
/// third outcome that neither routing nor the preflight describes.
#[test]
fn sandbox_explain_risky_mode_still_refuses_powershell() {
    let body = render_explain(&docker_config("risky"), None, Some("PowerShell"), None).unwrap();

    assert_eq!(decision(&body), "blocked_by_sandbox_mode");
    assert!(
        reason(&body).contains("cannot be sandbox-routed yet"),
        "{body}"
    );
}

/// With no isolation backend a session runs on the host under the `/sandbox`
/// toggle, which starts off. The workflow CLI has neither, so the sentence has
/// to say which of the two it is describing.
#[test]
fn sandbox_explain_without_a_backend_names_the_session_toggle() {
    let config = archon_core::sandbox::SandboxConfig::default();

    let body = render_explain(&config, None, Some("Bash"), None).unwrap();

    assert_eq!(decision(&body), "permission_preflight_only");
    assert!(
        reason(&body).contains("in a session the /sandbox toggle"),
        "{body}"
    );
    assert!(
        reason(&body).contains("`/sandbox on` would refuse it"),
        "explain did not say what the toggle would do to an execution tool: {body}"
    );
    assert!(
        reason(&body).contains("workflow CLI run has no toggle"),
        "the toggle claim is session-only and has to say so: {body}"
    );
}

/// `logical` installs no backend either, so the header and the tool section
/// have to agree about that rather than one of them implying a gate that is
/// not there.
#[test]
fn sandbox_explain_does_not_contradict_itself_about_logical() {
    let config = archon_core::sandbox::SandboxConfig {
        backend: "logical".into(),
        ..archon_core::sandbox::SandboxConfig::default()
    };

    let body = render_explain(&config, None, Some("Write"), None).unwrap();

    assert!(body.contains("Isolation: no isolation backend"), "{body}");
    assert!(
        reason(&body).contains("no isolation backend is configured"),
        "{body}"
    );
}

/// Writes used to be refused under `mode = "all"` because every backend
/// operated on the host filesystem. Each backend has its own now, so the
/// explanation has to say the write is routed rather than blocked — a stale
/// "blocked" here would send someone hunting for a restriction that is gone.
#[test]
fn sandbox_explain_all_mode_routes_write_tools_to_the_backend() {
    let body = render_explain(&docker_config("all"), None, Some("Write"), None).unwrap();

    assert_eq!(
        decision(&body),
        "route_to_sandbox_when_backend_ready",
        "explain still reports the pre-Phase-2 refusal: {body}"
    );
}
