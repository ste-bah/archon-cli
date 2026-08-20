//! The acceptance criterion for #201: one world, not two.
//!
//! > `Read` after a `Bash` write returns the written bytes under every backend
//! > and workspace mode.
//!
//! Everything else in the issue is machinery in service of that sentence. It is
//! also the one property a unit test cannot establish, because the failure it
//! guards against is precisely that the host and the container are different
//! filesystems — a fake world proves the plumbing, not the agreement.
//!
//! These are `#[ignore]` because they need a working Docker daemon and the
//! `ubuntu:24.04` image present locally (the backend runs with `--pull never`
//! on purpose, so it can never reach the network mid-session). Run them with:
//!
//! ```text
//! docker pull ubuntu:24.04
//! cargo test -p archon-core --test sandbox_docker_world -- --ignored --nocapture
//! ```
//!
//! Marked ignored rather than silently skipped when Docker is absent: a test
//! that quietly passes on a machine that could not run it reports coverage
//! nobody has.

use std::path::{Path, PathBuf};

use archon_core::sandbox::{DockerConfig, DockerFs, DockerSandboxBackend};
use archon_permissions::sandbox::{SandboxBackend, SandboxCommandRequest};
use archon_tools::filesystem::FileSystem;

fn docker_config() -> DockerConfig {
    DockerConfig {
        enabled: true,
        ..DockerConfig::default()
    }
}

fn request(working_dir: &Path, command: &str) -> SandboxCommandRequest {
    SandboxCommandRequest {
        command: command.to_string(),
        working_dir: working_dir.to_path_buf(),
        timeout_ms: 120_000,
        max_output_bytes: 64 * 1024,
        env: Vec::new(),
    }
}

/// The headline case: the container writes, the agent reads, same bytes.
#[tokio::test]
#[ignore = "requires a Docker daemon and the ubuntu:24.04 image"]
async fn read_returns_the_bytes_bash_wrote_in_the_container() {
    let dir = tempfile::tempdir().expect("tempdir");
    let backend = DockerSandboxBackend::new(docker_config(), "rw");
    let fs = DockerFs::new(dir.path());

    let result = backend
        .execute_bash(request(
            dir.path(),
            "printf 'written inside the container\\n' > /workspace/from_bash.txt",
        ))
        .await
        .expect("the docker backend executes bash");
    assert!(
        !result.is_error,
        "the container write itself failed: {}",
        result.content
    );

    let seen = fs
        .read_to_string(Path::new("/workspace/from_bash.txt"))
        .await
        .expect("the agent reads the path the container named");

    assert_eq!(seen, "written inside the container\n");
}

/// And the other direction, which is the one that actually bites: the agent
/// writes, then a command inside the container has to see it. A `Write` that
/// landed on the host while `Bash` ran in a container would pass the read-back
/// above and fail here.
#[tokio::test]
#[ignore = "requires a Docker daemon and the ubuntu:24.04 image"]
async fn bash_sees_the_bytes_the_agent_wrote() {
    let dir = tempfile::tempdir().expect("tempdir");
    let backend = DockerSandboxBackend::new(docker_config(), "rw");
    let fs = DockerFs::new(dir.path());

    fs.write(
        Path::new("/workspace/from_agent.txt"),
        b"written by the agent\n",
    )
    .await
    .expect("agent write");

    let result = backend
        .execute_bash(request(dir.path(), "cat /workspace/from_agent.txt"))
        .await
        .expect("the docker backend executes bash");

    assert!(!result.is_error, "{}", result.content);
    assert!(
        result.content.contains("written by the agent"),
        "the container could not see the agent's write: {}",
        result.content
    );
}

/// A path taken verbatim from container output must resolve, including one
/// several directories deep — the shape a compiler error or a `find` prints.
#[tokio::test]
#[ignore = "requires a Docker daemon and the ubuntu:24.04 image"]
async fn a_nested_path_printed_by_the_container_resolves() {
    let dir = tempfile::tempdir().expect("tempdir");
    let backend = DockerSandboxBackend::new(docker_config(), "rw");
    let fs = DockerFs::new(dir.path());

    let result = backend
        .execute_bash(request(
            dir.path(),
            "mkdir -p /workspace/src/deep && printf 'fn main() {}' > /workspace/src/deep/main.rs \
             && find /workspace -name main.rs",
        ))
        .await
        .expect("the docker backend executes bash");
    assert!(!result.is_error, "{}", result.content);

    let printed = result
        .content
        .lines()
        .map(str::trim)
        .find(|line| line.ends_with("main.rs"))
        .expect("find printed the path")
        .to_string();
    assert_eq!(
        printed, "/workspace/src/deep/main.rs",
        "the container names paths under the mount point"
    );

    let seen = fs
        .read_to_string(&PathBuf::from(&printed))
        .await
        .expect("the exact path the container printed");

    assert_eq!(seen, "fn main() {}");
}

/// A read-only workspace must actually be read-only in the container. If this
/// ever passes, `workspace_access = "ro"` is decoration.
#[tokio::test]
#[ignore = "requires a Docker daemon and the ubuntu:24.04 image"]
async fn a_readonly_workspace_refuses_a_container_write() {
    let dir = tempfile::tempdir().expect("tempdir");
    let backend = DockerSandboxBackend::new(docker_config(), "ro");

    let result = backend
        .execute_bash(request(
            dir.path(),
            "printf 'should not land' > /workspace/nope.txt",
        ))
        .await
        .expect("the docker backend executes bash");

    assert!(
        result.is_error || result.exit_code != Some(0),
        "a read-only mount accepted a write: {}",
        result.content
    );
    assert!(
        !dir.path().join("nope.txt").exists(),
        "the host file was created despite a read-only workspace"
    );
}
