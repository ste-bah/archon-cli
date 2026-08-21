use super::*;

#[test]
fn sandbox_status_shows_policy_fields() {
    let config = archon_core::sandbox::SandboxConfig {
        backend: "docker".into(),
        mode: "all".into(),
        workspace_access: "rw".into(),
        ..archon_core::sandbox::SandboxConfig::default()
    };

    let body = render_status(&config, true).unwrap();

    assert!(body.contains("Backend: docker"));
    assert!(body.contains("Mode: all"));
    assert!(body.contains("Workspace access: rw"));
    assert!(body.contains("normal permission rules still apply"));
    assert!(body.contains("Docker: enabled="));
    assert!(body.contains("mount_docker_socket=false"));
}

#[test]
fn sandbox_status_verbose_shows_openshell_safety_knobs() {
    let config = archon_core::sandbox::SandboxConfig {
        backend: "openshell".into(),
        openshell: archon_core::sandbox::OpenShellConfig {
            enabled: true,
            workspace_mode: "mirror".into(),
            ..archon_core::sandbox::OpenShellConfig::default()
        },
        ..archon_core::sandbox::SandboxConfig::default()
    };

    let body = render_status(&config, true).unwrap();

    assert!(body.contains("OpenShell: enabled=true"));
    assert!(body.contains("provider_injection=false"));
    assert!(body.contains("Provider routing: host-side"));
    assert!(body.contains("Claude Code spoofing"));
}

#[test]
fn sandbox_explain_rejects_unknown_backend_override() {
    let config = archon_core::sandbox::SandboxConfig::default();

    let error = render_explain(&config, Some("host".into()), None, None).unwrap_err();

    assert!(error.to_string().contains("sandbox.backend"));
}

#[test]
fn sandbox_explain_docker_shows_mount_network_and_redaction_policy() {
    let config = archon_core::sandbox::SandboxConfig {
        backend: "docker".into(),
        workspace_access: "scratch".into(),
        docker: archon_core::sandbox::DockerConfig {
            writable_paths: vec!["target".into()],
            network: "disabled".into(),
            ..archon_core::sandbox::DockerConfig::default()
        },
        ..archon_core::sandbox::SandboxConfig::default()
    };

    let body = render_explain(&config, None, None, None).unwrap();

    assert!(body.contains("workspace mounted read-only with ephemeral /scratch"));
    assert!(body.contains("Writable paths: target"));
    assert!(body.contains("Network policy: disabled"));
    assert!(body.contains("provider tokens"));
    assert!(body.contains("Docker socket"));
}

#[test]
fn sandbox_explain_openshell_keeps_provider_routing_host_side() {
    let config = archon_core::sandbox::SandboxConfig {
        backend: "openshell".into(),
        openshell: archon_core::sandbox::OpenShellConfig {
            enabled: true,
            workspace_mode: "mirror".into(),
            ..archon_core::sandbox::OpenShellConfig::default()
        },
        ..archon_core::sandbox::SandboxConfig::default()
    };

    let body = render_explain(&config, None, None, None).unwrap();

    assert!(body.contains("openshell mediated execution"));
    assert!(body.contains("provider_injection=false"));
    assert!(body.contains("Claude Code spoofing stays in Archon's provider runtime"));
    assert!(body.contains("host_shell_fallback=false"));
    assert!(body.contains("generated memory databases"));
}

#[test]
fn sandbox_explain_can_show_tool_routing_without_execution() {
    let config = archon_core::sandbox::SandboxConfig {
        backend: "openshell".into(),
        openshell: archon_core::sandbox::OpenShellConfig {
            enabled: true,
            ..archon_core::sandbox::OpenShellConfig::default()
        },
        ..archon_core::sandbox::SandboxConfig::default()
    };

    let body = render_explain(&config, None, Some("Bash"), Some("cargo test")).unwrap();

    assert!(body.contains("Tool: Bash"));
    assert!(body.contains("Decision: route_to_sandbox"));
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

    assert!(body.contains("Decision: unknown_tool"), "{body}");
    assert!(
        !body.contains("Decision: route_to_sandbox")
            && !body.contains("Decision: permission_preflight_only"),
        "explain fell through to a real-looking decision for an unknown name: {body}"
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
    assert!(body.contains("Decision: blocked_by_backend"), "{body}");
    assert!(
        body.contains("host handle the sandbox cannot redirect"),
        "the denial is not the gate's own message: {body}"
    );
}

#[test]
fn sandbox_explain_reports_the_backends_own_refusal_for_egress() {
    let body = render_explain(&docker_config("all"), None, Some("WebFetch"), None).unwrap();

    assert!(body.contains("Capability: egress"), "{body}");
    assert!(body.contains("Decision: blocked_by_backend"), "{body}");
    assert!(body.contains("leaves the machine"), "{body}");
}

/// The subtlety the whole command turns on: under the default mode the backend
/// is never asked about `Write`, so it must not be reported as routed. The
/// wrapper answers `Ok(())` for this call, and an explanation that read only
/// the `Ok` would say the sandbox allowed something it never saw.
#[test]
fn sandbox_explain_risky_mode_does_not_claim_the_backend_decided() {
    let body = render_explain(&docker_config("risky"), None, Some("Write"), None).unwrap();

    assert!(
        body.contains("Decision: permission_preflight_only"),
        "{body}"
    );
    assert!(body.contains("sandbox.mode = risky"), "{body}");
    assert!(
        !body.contains("Decision: route_to_sandbox"),
        "explain claimed the backend routed a tool it was never asked about: {body}"
    );
}

/// `shell` mode defers the same tools as `risky`; only `all` widens it. A
/// decision keyed on the backend alone would report the same thing for both,
/// and be wrong for one of them.
#[test]
fn sandbox_explain_follows_the_mode_rather_than_the_backend() {
    let risky = render_explain(&docker_config("risky"), None, Some("Edit"), None).unwrap();
    let all = render_explain(&docker_config("all"), None, Some("Edit"), None).unwrap();

    assert!(
        risky.contains("Decision: permission_preflight_only"),
        "{risky}"
    );
    assert!(all.contains("Decision: route_to_sandbox"), "{all}");
}

/// PowerShell is refused by the mode wrapper itself, before any backend — a
/// third outcome that neither `route_to_sandbox` nor `permission_preflight_only`
/// describes.
#[test]
fn sandbox_explain_risky_mode_still_refuses_powershell() {
    let body = render_explain(&docker_config("risky"), None, Some("PowerShell"), None).unwrap();

    assert!(body.contains("Decision: blocked_by_sandbox_mode"), "{body}");
    assert!(body.contains("cannot be sandbox-routed yet"), "{body}");
}

/// With no isolation backend the session runs on the host under the `/sandbox`
/// toggle, which starts off. Saying only "preflight" would leave out the one
/// gate that is actually there.
#[test]
fn sandbox_explain_without_a_backend_names_the_session_toggle() {
    let config = archon_core::sandbox::SandboxConfig::default();

    let body = render_explain(&config, None, Some("Bash"), None).unwrap();

    assert!(
        body.contains("Decision: permission_preflight_only"),
        "{body}"
    );
    assert!(body.contains("/sandbox toggle"), "{body}");
    assert!(
        body.contains("`/sandbox on` would refuse it"),
        "explain did not say what the toggle would do to an execution tool: {body}"
    );
}

/// Writes used to be refused under `mode = "all"` because every backend
/// operated on the host filesystem. Each backend has its own now, so the
/// explanation has to say the write is routed rather than blocked — a stale
/// "blocked" here would send someone hunting for a restriction that is gone.
#[test]
fn sandbox_explain_all_mode_routes_write_tools_to_the_backend() {
    let body = render_explain(&docker_config("all"), None, Some("Write"), None).unwrap();

    assert!(
        body.contains("Decision: route_to_sandbox"),
        "explain still reports the pre-Phase-2 refusal: {body}"
    );
}

#[test]
fn sandbox_test_is_detect_only() {
    let config = archon_core::sandbox::SandboxConfig::default();

    let body = sandbox_test::render_test(&config, Some("openshell".into())).unwrap();

    assert!(body.contains("Backend: openshell"));
    assert!(body.contains("No untrusted command was run"));
}

#[test]
fn sandbox_sessions_render_redacted_audit_rows() {
    let session = archon_learning::sandbox_sessions::SandboxSessionRecord::new(
        "sandbox-session-1",
        "openshell",
        "sandbox-profile-1",
        "configured",
        "2026-05-08T12:00:00Z",
    )
    .with_run_context(Some("run-1".into()), Some("reviewer".into()))
    .with_workspace(Some("mirror".into()), Some("local".into()))
    .with_transport(Some("openshell".into()), Some("gateway/[redacted]".into()));

    let body = render_sessions_table(&[session]);

    assert!(body.contains("sandbox-session-1"));
    assert!(body.contains("openshell"));
    assert!(body.contains("provider_injection"));
    assert!(body.contains("memory stores are redacted"));
}
