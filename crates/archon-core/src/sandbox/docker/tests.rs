use super::*;
use archon_permissions::ToolCapability;
use std::path::PathBuf;

#[test]
fn docker_defaults_are_safe() {
    let cfg = DockerConfig::default();

    assert!(!cfg.enabled);
    assert_eq!(cfg.binary, "docker");
    assert_eq!(cfg.network, "disabled");
    assert!(!cfg.privileged);
    assert!(!cfg.mount_docker_socket);
    assert!(!cfg.mount_home);
    assert!(cfg.env_allowlist.is_empty());
}

#[test]
fn doctor_flags_unsafe_docker_config() {
    let cfg = DockerConfig {
        enabled: true,
        privileged: true,
        mount_docker_socket: true,
        mount_home: true,
        ..DockerConfig::default()
    };

    let report = docker_doctor_report(&cfg, DockerProbe::found("Docker 27.0.0"));

    assert_eq!(report.status, DockerDoctorStatus::UnsafeConfig);
    assert!(render_docker_doctor_report(&report).contains("unsafe-config"));
}

#[test]
fn docker_run_args_default_to_no_network_and_readonly_workspace() {
    let cfg = DockerConfig {
        enabled: true,
        env_allowlist: vec!["RUST_LOG".into(), "ANTHROPIC_API_KEY".into()],
        ..DockerConfig::default()
    };
    let request = SandboxCommandRequest {
        command: "cargo test -p archon-core".into(),
        working_dir: PathBuf::from("/repo"),
        timeout_ms: 1_000,
        max_output_bytes: 1024,
        env: vec![
            ("RUST_LOG".into(), "debug".into()),
            ("ANTHROPIC_API_KEY".into(), "secret".into()),
        ],
        ..SandboxCommandRequest::default()
    };

    let args = docker_run_args(&cfg, "ro", &request);

    assert!(
        args.windows(2)
            .any(|pair| pair[0] == "--network" && pair[1] == "none")
    );
    assert!(
        args.iter()
            .any(|arg| arg.contains("dst=/workspace,readonly"))
    );
    assert!(
        args.windows(2)
            .any(|pair| pair[0] == "--env" && pair[1] == "RUST_LOG=debug")
    );
    assert!(!args.iter().any(|arg| arg.contains("ANTHROPIC_API_KEY")));
    assert!(args.iter().any(|arg| arg == "never"));
}

#[test]
fn docker_run_args_mount_explicit_writable_paths_over_readonly_workspace() {
    let cfg = DockerConfig {
        enabled: true,
        writable_paths: vec!["target".into(), "crates/archon-core".into()],
        ..DockerConfig::default()
    };
    let request = SandboxCommandRequest {
        command: "cargo test -p archon-core".into(),
        working_dir: PathBuf::from("/repo"),
        timeout_ms: 1_000,
        max_output_bytes: 1024,
        env: Vec::new(),
        ..SandboxCommandRequest::default()
    };

    let args = docker_run_args(&cfg, "ro", &request);

    assert!(
        args.iter()
            .any(|arg| arg == "type=bind,src=/repo,dst=/workspace,readonly")
    );
    assert!(
        args.iter()
            .any(|arg| arg == "type=bind,src=/repo/target,dst=/workspace/target")
    );
    assert!(args.iter().any(
        |arg| arg == "type=bind,src=/repo/crates/archon-core,dst=/workspace/crates/archon-core"
    ));
}

#[test]
fn docker_scratch_mode_adds_ephemeral_scratch_mount() {
    let cfg = DockerConfig {
        enabled: true,
        ..DockerConfig::default()
    };
    let request = SandboxCommandRequest {
        command: "echo scratch".into(),
        working_dir: PathBuf::from("/repo"),
        timeout_ms: 1_000,
        max_output_bytes: 1024,
        env: Vec::new(),
        ..SandboxCommandRequest::default()
    };

    let args = docker_run_args(&cfg, "scratch", &request);

    assert!(
        args.iter()
            .any(|arg| arg == "type=bind,src=/repo,dst=/workspace,readonly")
    );
    assert!(
        args.windows(2)
            .any(|pair| pair[0] == "--tmpfs" && pair[1] == "/scratch:rw,nosuid,size=512m")
    );
    assert!(
        args.windows(2)
            .any(|pair| pair[0] == "--env" && pair[1] == "ARCHON_SANDBOX_SCRATCH=/scratch")
    );
}

#[test]
fn docker_config_rejects_writable_path_escape() {
    let cfg = DockerConfig {
        writable_paths: vec!["../secret".into()],
        ..DockerConfig::default()
    };

    let error = cfg.validate().unwrap_err();

    assert!(error.contains("must not escape the workspace"));
}

#[test]
fn docker_backend_fails_closed_for_unsafe_config() {
    let backend = DockerSandboxBackend::new(
        DockerConfig {
            enabled: true,
            privileged: true,
            ..DockerConfig::default()
        },
        "rw",
        SandboxScope::Session,
    );

    let error = backend
        .check("Bash", ToolCapability::EXECUTION, &serde_json::json!({}))
        .unwrap_err();

    assert!(error.contains("privileged"));
}

#[test]
fn docker_backend_rejects_invalid_workspace_access() {
    let backend = DockerSandboxBackend::new(
        DockerConfig {
            enabled: true,
            ..DockerConfig::default()
        },
        "home",
        SandboxScope::Session,
    );

    let error = backend
        .check("Bash", ToolCapability::EXECUTION, &serde_json::json!({}))
        .unwrap_err();

    assert!(error.contains("workspace_access"));
}

mod terminals {
    //! #201 Phase 6: a terminal is a container with a TTY on it.

    use super::*;

    fn backend(workspace_access: &str) -> DockerSandboxBackend {
        DockerSandboxBackend::new(
            DockerConfig {
                enabled: true,
                ..DockerConfig::default()
            },
            workspace_access,
            SandboxScope::Session,
        )
    }

    fn request(cwd: &str) -> SandboxTerminalRequest {
        SandboxTerminalRequest {
            shell: None,
            workspace: PathBuf::from("/repo"),
            cwd: PathBuf::from(cwd),
        }
    }

    fn opened(backend: &DockerSandboxBackend, request: &SandboxTerminalRequest) -> Vec<String> {
        match backend.terminal(request) {
            SandboxTerminal::Open(command) => {
                assert_eq!(command.program, "docker");
                command.args
            }
            other => panic!("expected an open terminal, got {other:?}"),
        }
    }

    #[test]
    fn the_terminal_container_is_the_command_container_plus_a_tty() {
        let backend = backend("rw");
        let command_args = docker_run_args(
            &DockerConfig {
                enabled: true,
                ..DockerConfig::default()
            },
            "rw",
            &SandboxCommandRequest {
                command: "true".into(),
                working_dir: PathBuf::from("/repo"),
                timeout_ms: 1_000,
                max_output_bytes: 1_024,
                env: Vec::new(),
                ..SandboxCommandRequest::default()
            },
        );

        let args = opened(&backend, &request("/repo"));

        for shared in [
            "--rm",
            "no-new-privileges",
            "ALL",
            "--pids-limit",
            "type=bind,src=/repo,dst=/workspace",
        ] {
            assert!(
                args.contains(&shared.to_string()),
                "a terminal must not get a laxer container than Bash does; missing {shared}"
            );
            assert!(command_args.contains(&shared.to_string()));
        }
        assert!(args.contains(&"--interactive".to_string()));
        assert!(args.contains(&"--tty".to_string()));
        assert_eq!(
            args.last().map(String::as_str),
            Some("/bin/bash"),
            "the shell is the container's command"
        );
    }

    /// The container needs `TERM` set explicitly: the docker CLI's own
    /// environment does not cross into it.
    #[test]
    fn the_container_shell_is_told_it_has_a_terminal() {
        let args = opened(&backend("rw"), &request("/repo"));

        assert!(
            args.contains(&"TERM=xterm-256color".to_string()),
            "{args:?}"
        );
    }

    #[test]
    fn a_subdirectory_becomes_the_containers_workdir() {
        let args = opened(&backend("rw"), &request("/repo/crates/archon-core"));

        let workdir = args
            .windows(2)
            .find(|pair| pair[0] == "--workdir")
            .expect("a workdir");

        assert_eq!(workdir[1], "/workspace/crates/archon-core");
    }

    /// Starting at the workspace root instead would answer a question nobody
    /// asked, in a directory the caller did not choose.
    #[test]
    fn a_directory_outside_the_workspace_is_refused_by_name() {
        let SandboxTerminal::Refused(reason) = backend("rw").terminal(&request("/etc")) else {
            panic!("a path the container cannot see must not silently become /workspace");
        };

        assert!(reason.contains("/etc"), "{reason}");
        assert!(reason.contains("outside the sandbox workspace"), "{reason}");
    }

    /// On Windows the host default shell is PowerShell, which no Linux image
    /// has. Asking for it must explain that rather than fail inside the PTY.
    #[test]
    fn a_windows_only_shell_is_refused_with_the_reason() {
        let mut request = request("/repo");
        request.shell = Some("powershell".into());

        let SandboxTerminal::Refused(reason) = backend("rw").terminal(&request) else {
            panic!("a Linux container has no PowerShell");
        };

        assert!(reason.contains("Linux container"), "{reason}");
        assert!(reason.contains("bash"), "{reason}");
    }

    /// A default request must still open, because on Windows the host default
    /// is PowerShell and resolving it before the backend was asked would refuse
    /// every terminal opened without an explicit shell.
    #[test]
    fn an_unspecified_shell_becomes_the_containers_bash() {
        match backend("rw").terminal(&request("/repo")) {
            SandboxTerminal::Open(command) => assert_eq!(command.shell, "bash"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn an_unsafe_config_refuses_a_terminal_rather_than_opening_one() {
        let backend = DockerSandboxBackend::new(
            DockerConfig {
                enabled: true,
                privileged: true,
                ..DockerConfig::default()
            },
            "rw",
            SandboxScope::Session,
        );

        let SandboxTerminal::Refused(reason) = backend.terminal(&request("/repo")) else {
            panic!("a privileged container must not host a terminal either");
        };

        assert!(reason.contains("privileged"), "{reason}");
    }

    /// Docker can put a TTY on a container, so the gate must let a terminal
    /// through to `terminal()` rather than deciding against it. Asserted by
    /// class, not by name: the names are exactly what stopped deciding
    /// anything.
    #[test]
    fn a_terminal_reaches_the_backend_rather_than_being_refused_by_the_gate() {
        let backend = backend("rw");

        assert!(
            backend
                .check(
                    "TerminalCreate",
                    archon_permissions::ToolCapability::TERMINAL,
                    &serde_json::json!({})
                )
                .is_ok(),
            "docker attaches a TTY to a container, so the gate must not refuse it"
        );
    }
}
