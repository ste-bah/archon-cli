//! Tests for the sandbox configuration surface.
//!
//! A sibling file rather than an inline module: mod.rs is at the 500-line
//! ceiling and these are the part that grows fastest.

use super::*;

#[test]
fn sandbox_config_deserializes_openshell_section() {
    let cfg: SandboxConfig = toml::from_str(
        r#"
        backend = "openshell"
        mode = "all"
        scope = "session"
        workspace_access = "rw"

        [openshell]
        enabled = true
        workspace_mode = "remote"
        gateway = "team-gateway"
        remote_workdir = "/workspace/team"
        policy = "locked-down"
        providers = ["ssh"]
        gpu = true
        "#,
    )
    .unwrap();

    assert_eq!(cfg.backend, "openshell");
    assert_eq!(cfg.backend_kind().unwrap(), SandboxBackendKind::OpenShell);
    assert_eq!(cfg.policy().unwrap().mode, "all");
    assert_eq!(cfg.policy().unwrap().workspace_access, "rw");
    assert!(cfg.openshell.enabled);
    assert_eq!(cfg.openshell.workspace_mode, "remote");
    assert_eq!(cfg.openshell.gateway.as_deref(), Some("team-gateway"));
    assert_eq!(
        cfg.openshell.remote_workdir.as_deref(),
        Some("/workspace/team")
    );
    assert!(cfg.openshell.gpu);
    assert!(!cfg.openshell.provider_injection);
    assert!(!cfg.openshell.host_shell_fallback);
}

#[test]
fn sandbox_config_deserializes_ssh_section() {
    let cfg: SandboxConfig = toml::from_str(
        r#"
        backend = "ssh"
        mode = "all"

        [ssh]
        enabled = true
        host = "sandbox.example"
        user = "archon"
        port = 2222
        workspace_mode = "remote"
        "#,
    )
    .unwrap();

    assert_eq!(cfg.backend, "ssh");
    assert_eq!(cfg.backend_kind().unwrap(), SandboxBackendKind::Ssh);
    assert!(cfg.ssh.enabled);
    assert_eq!(cfg.ssh.host.as_deref(), Some("sandbox.example"));
    assert_eq!(cfg.ssh.user.as_deref(), Some("archon"));
    assert_eq!(cfg.ssh.port, 2222);
    assert_eq!(cfg.ssh.workspace_mode, "remote");
    assert!(cfg.ssh.host_key_checking);
    assert!(!cfg.ssh.host_shell_fallback);
}

/// A backend that never leaves the host must not pay for a filesystem it
/// does not need — and `None` here is what keeps behaviour identical for
/// everyone not running a sandbox.
#[test]
fn a_host_bound_backend_gets_no_filesystem_of_its_own() {
    for backend in ["disabled", "logical"] {
        let cfg = SandboxConfig {
            backend: backend.into(),
            ..SandboxConfig::default()
        };

        assert!(
            sandbox_filesystem(&cfg, std::path::Path::new("/tree"))
                .expect("host-bound backends always resolve")
                .is_none(),
            "{backend} should run on the host filesystem"
        );
    }
}

/// Docker must get a *translating* filesystem, not merely some filesystem.
/// Asserting `is_some()` would pass if the factory handed back `LocalFs`,
/// which is the bug: `Bash` reports `/workspace/...` and `Read` would then
/// look for that path on the host.
#[tokio::test]
async fn docker_gets_a_filesystem_that_translates_container_paths() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("in_workspace.txt"), "mounted").expect("write");
    let cfg = SandboxConfig {
        backend: "docker".into(),
        ..SandboxConfig::default()
    };

    let fs = sandbox_filesystem(&cfg, dir.path())
        .expect("docker resolves")
        .expect("docker has a world of its own");

    assert_eq!(
        fs.read(std::path::Path::new("/workspace/in_workspace.txt"))
            .await
            .expect("the path the container would name"),
        b"mounted"
    );
}

/// A remote backend that cannot be routed must fail loudly. Returning the
/// host here would be the silent split this whole issue exists to close.
#[test]
fn an_unroutable_remote_backend_is_an_error_not_a_fallback_to_the_host() {
    let cfg = SandboxConfig {
        backend: "ssh".into(),
        ..SandboxConfig::default()
    };

    let error = sandbox_filesystem(&cfg, std::path::Path::new("/tree"))
        .expect_err("a disabled ssh backend cannot supply a workspace");

    assert!(!error.is_empty(), "the failure has to say why");
}

#[test]
fn sandbox_config_rejects_unknown_backend() {
    let cfg = SandboxConfig {
        backend: "host".into(),
        ..SandboxConfig::default()
    };

    let error = cfg.validate().unwrap_err();

    assert!(error.contains("sandbox.backend"));
}

/// A configuration whose scope the operator chose deliberately.
fn config(backend: &str, scope: &str) -> SandboxConfig {
    SandboxConfig {
        backend: backend.into(),
        scope: scope.into(),
        scope_explicit: true,
        docker: DockerConfig {
            enabled: true,
            ..DockerConfig::default()
        },
        ..SandboxConfig::default()
    }
}

fn support(config: &SandboxConfig) -> Option<SandboxScopeSupport> {
    match config.scope_decision().expect("resolves") {
        ScopeDecision::Honoured { support, .. } => Some(support),
        ScopeDecision::NotApplicable => None,
        ScopeDecision::FellBack { reason, .. } => {
            panic!("expected the configured scope to be honoured, got a fallback: {reason}")
        }
    }
}

/// The failure this whole change exists to stop repeating: a scope that
/// loads cleanly and is then quietly not honoured. openshell destroys its
/// sandbox after every command, so a longer lifetime has to be refused where
/// the operator can see it.
#[test]
fn a_backend_that_cannot_hold_a_sandbox_refuses_the_scope_at_config_load() {
    for scope in ["session", "turn"] {
        let error = config("openshell", scope)
            .validate()
            .expect_err("this configuration must not load");

        assert!(error.contains("--no-keep"), "{error}");
        assert!(
            error.contains("sandbox.scope = \"tool\""),
            "the refusal has to name the setting that works: {error}"
        );
    }
    config("openshell", "tool")
        .validate()
        .expect("`tool` is exactly what this backend does");
}

#[test]
fn docker_accepts_every_scope_and_says_which_hold_a_container() {
    for scope in ["session", "turn"] {
        assert_eq!(
            support(&config("docker", scope)),
            Some(SandboxScopeSupport::Held),
            "{scope} should hold a container open"
        );
    }
    assert_eq!(
        support(&config("docker", "tool")),
        Some(SandboxScopeSupport::PerCommand)
    );
}

/// ssh's world is a machine Archon neither creates nor destroys, so no scope
/// can be wrong and none can be honoured either — state simply survives.
#[test]
fn a_backend_whose_world_outlives_archon_reports_no_lifetime_to_manage() {
    for scope in ["session", "turn", "tool"] {
        assert_eq!(
            support(&config("ssh", scope)),
            Some(SandboxScopeSupport::Durable)
        );
    }
}

/// A backend that creates no world has no lifetime for a scope to name, and
/// must not be made to answer for one.
#[test]
fn a_non_isolating_backend_has_no_scope_to_support() {
    for backend in ["disabled", "logical"] {
        assert_eq!(support(&config(backend, "session")), None);
    }
}

/// Breaking someone on a value they chose is one thing; breaking them on a
/// struct default they never wrote is another. `scope` defaults to
/// `session`, so before this a bare `[sandbox] backend = "openshell"` failed
/// the *whole* configuration over a key that was not in the file.
#[test]
fn a_scope_nobody_set_falls_back_instead_of_failing_the_configuration() {
    let never_set = SandboxConfig {
        backend: "openshell".into(),
        ..SandboxConfig::default()
    };
    assert!(!never_set.scope_explicit, "the default is not a choice");

    never_set
        .validate()
        .expect("a configuration that names no scope must still load");

    let ScopeDecision::FellBack {
        scope,
        from,
        reason,
    } = never_set.scope_decision().expect("resolves")
    else {
        panic!("openshell cannot keep the default `session`, so this must fall back");
    };
    assert_eq!(scope, archon_permissions::SandboxScope::Tool);
    assert_eq!(from, archon_permissions::SandboxScope::Session);
    assert!(reason.contains("--no-keep"), "{reason}");
}

/// The distinction is the whole point: the same value refuses when chosen.
#[test]
fn the_same_scope_still_refuses_when_the_operator_chose_it() {
    let chosen = SandboxConfig {
        scope_explicit: true,
        ..SandboxConfig {
            backend: "openshell".into(),
            ..SandboxConfig::default()
        }
    };

    assert!(
        chosen.validate().is_err(),
        "an explicitly chosen lifetime the backend cannot keep must refuse"
    );
}

/// Which half of the distinction applies is decided by the file, so the
/// deserializer has to record whether the key was there at all.
#[test]
fn deserialization_records_whether_the_scope_key_was_present() {
    let absent: SandboxConfig = toml::from_str("backend = \"docker\"").expect("parses");
    assert!(!absent.scope_explicit);
    assert_eq!(absent.scope, "session", "the default still applies");

    let present: SandboxConfig =
        toml::from_str("backend = \"docker\"\nscope = \"session\"").expect("parses");
    assert!(
        present.scope_explicit,
        "a key written in the file is a choice, even when it matches the default"
    );
    assert_eq!(present.mode, "risky", "unrelated defaults still fill in");
}

/// The templates ship with `backend` set to something and `scope` commented
/// out. Copying one and switching the backend to openshell must not brick
/// the whole configuration.
#[test]
fn the_shipped_template_loads_under_every_backend() {
    let template = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config.toml"),
    )
    .expect("the shipped template is part of the repo");
    let full: crate::config::ArchonConfig = toml::from_str(&template).expect("template parses");

    assert!(
        !full.sandbox.scope_explicit,
        "config.toml must leave `scope` commented out, or a user who switches \
         backend inherits a lifetime they never chose and cannot load"
    );
    for backend in ["disabled", "logical", "docker", "ssh", "openshell"] {
        let switched = SandboxConfig {
            backend: backend.into(),
            ..full.sandbox.clone()
        };
        switched
            .validate()
            .unwrap_or_else(|error| panic!("template + {backend} does not load: {error}"));
    }
}
