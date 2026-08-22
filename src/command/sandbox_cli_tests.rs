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

/// `Scope: session` echoes the config file, and for the whole life of that field
/// it was not what happened. Status has to say what the backend actually does
/// with it — that is the question an operator whose build re-downloads its
/// dependencies every command is really asking.
#[test]
fn sandbox_status_says_what_the_backend_does_with_the_configured_scope() {
    let held = archon_core::sandbox::SandboxConfig {
        backend: "docker".into(),
        scope: "session".into(),
        docker: archon_core::sandbox::DockerConfig {
            enabled: true,
            ..archon_core::sandbox::DockerConfig::default()
        },
        ..archon_core::sandbox::SandboxConfig::default()
    };

    let body = render_status(&held, true).unwrap();
    assert!(body.contains("Scope: session"), "{body}");
    assert!(
        body.contains("Sandbox lifetime: one sandbox held open"),
        "status must say the container is reused, not just that a scope was set: {body}"
    );
    assert!(
        body.contains("Docker container max age: 14400s"),
        "the bound that stops a leaked container living forever is not shown: {body}"
    );

    let per_command = archon_core::sandbox::SandboxConfig {
        scope: "tool".into(),
        ..held
    };
    assert!(
        render_status(&per_command, false)
            .unwrap()
            .contains("Sandbox lifetime: a sandbox built and destroyed per command"),
        "`tool` must read differently from `session`, or the line is decoration"
    );
}

/// The reachable half of "the backend cannot keep this scope": a default nobody
/// chose, which falls back rather than failing the configuration. Status has to
/// show both what was configured and what is actually happening, or the fallback
/// is silent.
#[test]
fn sandbox_status_shows_a_scope_that_fell_back_and_says_why() {
    let config = archon_core::sandbox::SandboxConfig {
        backend: "openshell".into(),
        ..archon_core::sandbox::SandboxConfig::default()
    };

    let body = render_status(&config, false).unwrap();

    assert!(
        body.contains("Scope: session"),
        "the configured value: {body}"
    );
    assert!(
        body.contains("Sandbox lifetime: tool"),
        "the effective value has to be shown next to it: {body}"
    );
    assert!(body.contains("--no-keep"), "and the reason: {body}");
    assert!(
        body.contains("Set sandbox.scope explicitly"),
        "an operator has to be told how to take the decision back: {body}"
    );
}

/// An explicitly chosen scope the backend cannot keep never reaches
/// `render_status` in a real run — config load refuses it first — but status
/// must not paper over one either.
#[test]
fn sandbox_status_names_a_scope_the_operator_chose_and_the_backend_refuses() {
    let config = archon_core::sandbox::SandboxConfig {
        backend: "openshell".into(),
        scope: "session".into(),
        scope_explicit: true,
        ..archon_core::sandbox::SandboxConfig::default()
    };

    let body = render_status(&config, false).unwrap();

    assert!(body.contains("Sandbox lifetime: unsupported:"), "{body}");
    assert!(body.contains("--no-keep"), "{body}");
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
