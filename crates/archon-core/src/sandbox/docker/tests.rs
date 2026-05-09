use super::*;
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
fn docker_backend_fails_closed_for_unsafe_config() {
    let backend = DockerSandboxBackend::new(
        DockerConfig {
            enabled: true,
            privileged: true,
            ..DockerConfig::default()
        },
        "rw",
    );

    let error = backend.check("Bash", &serde_json::json!({})).unwrap_err();

    assert!(error.contains("privileged"));
}
