#![cfg(unix)]
use super::*;
use archon_permissions::sandbox::{SandboxBackend, SandboxCommandRequest, SandboxScope};
use std::os::unix::fs::PermissionsExt;

async fn probe(class: &str, safe: bool, name: &str) {
    if std::env::var("ARCHON_TEST_SECRET").as_deref() != Ok("sk-test-355") {
        let status = archon_shell::spawn::command(std::env::current_exe().unwrap())
            .args([
                "--exact",
                &format!("sandbox::agent_environment_tests::{name}"),
                "--nocapture",
            ])
            .env("ARCHON_TEST_SECRET", "sk-test-355")
            .env("LANG", "C")
            .env("TMPDIR", std::env::temp_dir())
            .status()
            .unwrap();
        assert!(status.success(), "transport environment regression: {name}");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let binary = dir.path().join("transport");
    std::fs::write(&binary, "#!/bin/sh\nif [ -z \"${ARCHON_TEST_SECRET+x}\" ]; then echo withheld; fi\nif [ -n \"$PATH\" ] && [ -n \"$HOME\" ] && [ \"$LANG\" = C ] && [ -n \"$TMPDIR\" ]; then echo safe; fi\n").unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    let binary = binary.to_str().unwrap().to_owned();
    let backend: Box<dyn SandboxBackend> = match class {
        "ssh" => Box::new(SshSandboxBackend::new(SshConfig {
            enabled: true,
            binary,
            host: Some("example.invalid".into()),
            remote_workdir: Some("/workspace".into()),
            ..Default::default()
        })),
        "docker" => Box::new(DockerSandboxBackend::new(
            DockerConfig {
                enabled: true,
                binary,
                ..Default::default()
            },
            "rw",
            SandboxScope::Tool,
        )),
        "openshell" => Box::new(OpenShellSandboxBackend::new(OpenShellConfig {
            enabled: true,
            binary,
            ..Default::default()
        })),
        _ => unreachable!(),
    };
    let result = backend
        .execute_bash(SandboxCommandRequest {
            command: "true".into(),
            working_dir: dir.path().into(),
            timeout_ms: 30_000,
            max_output_bytes: 4096,
            env: Vec::new(),
            session_id: "test".into(),
            turn_id: None,
        })
        .await
        .unwrap();
    assert!(!result.is_error, "transport launch failed");
    assert!(
        result
            .content
            .contains(if safe { "safe" } else { "withheld" }),
        "required environment property absent"
    );
}
macro_rules! regression {
    ($name:ident, $class:literal, $safe:literal) => {
        #[tokio::test]
        async fn $name() {
            probe($class, $safe, stringify!($name)).await;
        }
    };
}
regression!(agent_environment_ssh_secret, "ssh", false);
regression!(agent_environment_ssh_safe, "ssh", true);
regression!(agent_environment_docker_secret, "docker", false);
regression!(agent_environment_docker_safe, "docker", true);
regression!(agent_environment_openshell_secret, "openshell", false);
regression!(agent_environment_openshell_safe, "openshell", true);

#[test]
fn host_written_remote_token_is_in_the_read_boundary() {
    assert!(
        archon_shell::read_boundary::operator_paths().contains(&crate::remote::auth::token_path()),
        "host remote bearer token must be protected"
    );
}

#[test]
fn host_written_plan_approval_secret_is_in_the_read_boundary() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("approval.secret");
    let _ = archon_session::plan::load_or_create_approval_secret(&path).unwrap();
    assert!(
        archon_shell::read_boundary::operator_paths().contains(&path),
        "host-written signing secret must be protected"
    );
}
