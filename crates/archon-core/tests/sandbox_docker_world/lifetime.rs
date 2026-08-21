//! `sandbox.scope`, against a real daemon.
//!
//! The claim being tested is that a container is *reused* — and that is exactly
//! the claim a fake cannot support. So every assertion here rests on two facts
//! the container itself supplies and no mock could fabricate:
//!
//! - `/etc/hostname` inside a container is that container's own id, so two
//!   commands that report the same one ran in the same container.
//! - `/tmp` is a tmpfs belonging to the container and is *not* the bind mount,
//!   so a file written there survives exactly as long as the container does.
//!   This is the state the old per-command backend destroyed: not the workspace,
//!   which the mount preserves, but everything beside it — `~/.cargo/registry`,
//!   `~/.npm`, apt lists, `/tmp`.
//!
//! `#[ignore]` for the same reason as the rest of the suite: these need a
//! working Docker daemon and `ubuntu:24.04` present locally. A module of
//! `sandbox_docker_world.rs` rather than a file beside it, so both stay under
//! the 500-line ceiling without splitting the suite into two test binaries.
//!
//! ```text
//! docker pull ubuntu:24.04
//! cargo test -p archon-core --test sandbox_docker_world -- --ignored --nocapture
//! ```

use std::path::Path;

use archon_core::sandbox::{DockerConfig, DockerSandboxBackend};
use archon_permissions::sandbox::{SandboxBackend, SandboxCommandRequest, SandboxScope};

/// Teardown, on the helpers defined below. A sibling file only because of the
/// 500-line ceiling.
#[path = "cleanup.rs"]
mod cleanup;

fn docker_config() -> DockerConfig {
    DockerConfig {
        enabled: true,
        // Two minutes rather than the four-hour default, because this is the
        // mechanism that has to save us when a test panics before teardown.
        container_max_age_secs: 120,
        ..DockerConfig::default()
    }
}

fn backend(scope: SandboxScope) -> DockerSandboxBackend {
    DockerSandboxBackend::new(docker_config(), "rw", scope)
}

fn request(working_dir: &Path, session: &str, turn: &str, command: &str) -> SandboxCommandRequest {
    SandboxCommandRequest {
        command: command.to_string(),
        working_dir: working_dir.to_path_buf(),
        timeout_ms: 120_000,
        max_output_bytes: 64 * 1024,
        env: Vec::new(),
        session_id: session.to_string(),
        turn_id: Some(turn.to_string()),
    }
}

/// Run a command and return its output, failing loudly rather than letting a
/// broken container look like a missing marker.
async fn run(backend: &DockerSandboxBackend, request: SandboxCommandRequest) -> String {
    let result = backend
        .execute_bash(request)
        .await
        .expect("the docker backend executes bash");
    assert!(!result.is_error, "command failed: {}", result.content);
    result.content
}

/// The container this command ran in. `/etc/hostname` is the container's own id.
const WHICH_CONTAINER: &str = "cat /etc/hostname";

fn container_id(output: &str) -> String {
    let id = output
        .lines()
        .map(str::trim)
        .find(|line| line.len() == 12 && line.chars().all(|c| c.is_ascii_hexdigit()))
        .unwrap_or_else(|| panic!("no container id in output: {output}"));
    id.to_string()
}

fn container_exists(id: &str) -> bool {
    std::process::Command::new("docker")
        .args(["inspect", "--format", "{{.Id}}", id])
        .output()
        .is_ok_and(|output| output.status.success())
}

/// Whether a container is discoverable by the label reaping filters on. A held
/// container that is not is one nothing can ever find again.
fn is_discoverable_by_label(id: &str) -> bool {
    std::process::Command::new("docker")
        .args([
            "ps",
            "--quiet",
            "--no-trunc",
            "--filter",
            "label=archon.sandbox=1",
        ])
        .output()
        .is_ok_and(|output| {
            output.status.success()
                && String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .any(|line| line.trim().starts_with(id))
        })
}

fn remove_container(id: &str) {
    let _ = std::process::Command::new("docker")
        .args(["rm", "--force", id])
        .output();
}

/// Removes a container however the test ends, including a panicking assertion.
struct Removed(String);

impl Drop for Removed {
    fn drop(&mut self) {
        remove_container(&self.0);
    }
}

/// The headline claim. Under the old backend the second command ran in a
/// container built after the first one was destroyed, so the marker was gone and
/// so was every build cache beside it.
#[tokio::test]
#[ignore = "requires a Docker daemon and the ubuntu:24.04 image"]
async fn a_session_scoped_container_is_reused_across_commands() {
    let dir = tempfile::tempdir().expect("tempdir");
    let backend = backend(SandboxScope::Session);

    let first = run(
        &backend,
        request(
            dir.path(),
            "reuse",
            "reuse#1",
            &format!("printf held > /tmp/marker; {WHICH_CONTAINER}"),
        ),
    )
    .await;
    let second = run(
        &backend,
        request(
            dir.path(),
            "reuse",
            "reuse#2",
            &format!("cat /tmp/marker; printf '\\n'; {WHICH_CONTAINER}"),
        ),
    )
    .await;

    assert!(
        second.contains("held"),
        "the container was rebuilt between commands, so everything outside the \
         workspace mount was destroyed with it: {second}"
    );
    assert_eq!(
        container_id(&first),
        container_id(&second),
        "two different containers answered"
    );
    assert!(
        is_discoverable_by_label(&container_id(&first)),
        "the held container does not carry the label reaping filters on, so \
         nothing could ever find it again if this process died"
    );
}

/// The other half of the branch. If this ever fails, `scope` is being ignored
/// again — in the opposite direction.
#[tokio::test]
#[ignore = "requires a Docker daemon and the ubuntu:24.04 image"]
async fn a_tool_scoped_backend_destroys_its_world_after_every_command() {
    let dir = tempfile::tempdir().expect("tempdir");
    let backend = backend(SandboxScope::Tool);

    let first = run(
        &backend,
        request(
            dir.path(),
            "per-tool",
            "per-tool#1",
            &format!("printf held > /tmp/marker; {WHICH_CONTAINER}"),
        ),
    )
    .await;
    let second = run(
        &backend,
        request(
            dir.path(),
            "per-tool",
            "per-tool#1",
            &format!("cat /tmp/marker 2>/dev/null; printf '\\n'; {WHICH_CONTAINER}"),
        ),
    )
    .await;

    assert!(
        !second.contains("held"),
        "`tool` scope kept a container alive between commands: {second}"
    );
    assert_ne!(container_id(&first), container_id(&second));
}

/// A fan-out sharing one working directory now shares one container — which is
/// a *tightening*, and the reason it is worth a test of its own.
///
/// Ten concurrent subagents in one tree used to get ten containers and ten times
/// the memory and pid budget. They now share one container's `--memory 2g` and
/// `--pids-limit 256`. That is the consolidation this change buys, and it is
/// also how a fan-out that fitted before can stop fitting: the limits are
/// per-container and there is now one container where there were ten.
///
/// Also the only test that drives concurrent `container_for` calls for the same
/// key, so it is what would catch a pool that deadlocked or double-created under
/// contention.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires a Docker daemon and the ubuntu:24.04 image"]
async fn a_same_directory_fan_out_shares_exactly_one_container() {
    let dir = tempfile::tempdir().expect("tempdir");
    let backend = std::sync::Arc::new(backend(SandboxScope::Session));

    let mut fan_out = Vec::new();
    for agent in 0..10 {
        let backend = std::sync::Arc::clone(&backend);
        let workspace = dir.path().to_path_buf();
        fan_out.push(tokio::spawn(async move {
            let result = backend
                .execute_bash(request(
                    &workspace,
                    "fan-out",
                    "fan-out#1",
                    &format!("printf 'agent {agent}\\n'; {WHICH_CONTAINER}"),
                ))
                .await
                .expect("the docker backend executes bash");
            assert!(!result.is_error, "agent {agent} failed: {}", result.content);
            container_id(&result.content)
        }));
    }

    let mut ids = std::collections::BTreeSet::new();
    for agent in fan_out {
        ids.insert(agent.await.expect("no agent panicked"));
    }

    assert_eq!(
        ids.len(),
        1,
        "ten concurrent commands on one working directory produced {} containers; \
         the pool either raced and double-created, or is not sharing at all: {ids:?}",
        ids.len()
    );
}

/// `workspace_access = "scratch"` promises "a read-only workspace plus somewhere
/// to write". Under a container per command that promise could not be kept:
/// `/scratch` was destroyed by the very command that wrote to it, so the mount
/// existed and was useless.
#[tokio::test]
#[ignore = "requires a Docker daemon and the ubuntu:24.04 image"]
async fn scratch_space_survives_the_command_that_wrote_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let backend = DockerSandboxBackend::new(docker_config(), "scratch", SandboxScope::Session);

    run(
        &backend,
        request(
            dir.path(),
            "scratch",
            "scratch#1",
            "printf staged > \"$ARCHON_SANDBOX_SCRATCH/work.txt\"",
        ),
    )
    .await;
    let second = run(
        &backend,
        request(
            dir.path(),
            "scratch",
            "scratch#2",
            "cat \"$ARCHON_SANDBOX_SCRATCH/work.txt\"",
        ),
    )
    .await;

    assert!(
        second.contains("staged"),
        "scratch space did not survive the command that wrote it: {second}"
    );
}

/// A worktree-isolated subagent mounts a different tree and inherits its
/// parent's session and turn ids. If the working directory were not in the key,
/// the two would land in one container — two agents in a single world, each
/// believing it was isolated.
#[tokio::test]
#[ignore = "requires a Docker daemon and the ubuntu:24.04 image"]
async fn two_working_directories_never_share_a_container() {
    let parent_tree = tempfile::tempdir().expect("tempdir");
    let worktree = tempfile::tempdir().expect("tempdir");
    let backend = backend(SandboxScope::Session);

    let parent = run(
        &backend,
        request(
            parent_tree.path(),
            "shared",
            "shared#1",
            &format!("printf parent > /tmp/marker; {WHICH_CONTAINER}"),
        ),
    )
    .await;
    let child = run(
        &backend,
        request(
            worktree.path(),
            "shared",
            "shared#1",
            &format!("cat /tmp/marker 2>/dev/null; printf '\\n'; {WHICH_CONTAINER}"),
        ),
    )
    .await;

    assert!(
        !child.contains("parent"),
        "a second working directory was handed the first one's container: {child}"
    );
    assert_ne!(container_id(&parent), container_id(&child));
}

/// `turn` has to end at the turn boundary and the old container has to go, or it
/// is `session` under another name plus a leak.
#[tokio::test]
#[ignore = "requires a Docker daemon and the ubuntu:24.04 image"]
async fn a_new_turn_gets_a_new_container_and_the_previous_one_is_torn_down() {
    let dir = tempfile::tempdir().expect("tempdir");
    let backend = backend(SandboxScope::Turn);

    let first_turn = run(
        &backend,
        request(
            dir.path(),
            "turns",
            "turns#1",
            &format!("printf held > /tmp/marker; {WHICH_CONTAINER}"),
        ),
    )
    .await;
    let first_id = container_id(&first_turn);

    let same_turn = run(
        &backend,
        request(
            dir.path(),
            "turns",
            "turns#1",
            &format!("cat /tmp/marker; printf '\\n'; {WHICH_CONTAINER}"),
        ),
    )
    .await;
    assert!(
        same_turn.contains("held"),
        "a second command in the same turn got a different container: {same_turn}"
    );

    let next_turn = run(
        &backend,
        request(
            dir.path(),
            "turns",
            "turns#2",
            &format!("cat /tmp/marker 2>/dev/null; printf '\\n'; {WHICH_CONTAINER}"),
        ),
    )
    .await;

    assert!(
        !next_turn.contains("held"),
        "the previous turn's container survived the turn boundary: {next_turn}"
    );
    assert_ne!(first_id, container_id(&next_turn));
    assert!(
        !container_exists(&first_id),
        "the previous turn's container is still running; `turn` scope leaks one \
         container per turn"
    );
}

/// Environment moves from `--env` on `docker run` to `--env` on `docker exec`
/// when a container is held. The unit tests pin the arguments; this proves the
/// daemon accepts them and that the allowlist survived the move — a held
/// container must not become a way to smuggle in a variable the per-command path
/// would have dropped.
#[tokio::test]
#[ignore = "requires a Docker daemon and the ubuntu:24.04 image"]
async fn a_held_container_gets_allowlisted_environment_and_not_credentials() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config = DockerConfig {
        env_allowlist: vec!["RUST_LOG".into(), "ANTHROPIC_API_KEY".into()],
        ..docker_config()
    };
    let backend = DockerSandboxBackend::new(config, "rw", SandboxScope::Session);

    let mut command = request(
        dir.path(),
        "env",
        "env#1",
        "printf 'LOG=%s SECRET=%s\\n' \"${RUST_LOG:-unset}\" \"${ANTHROPIC_API_KEY:-unset}\"",
    );
    command.env = vec![
        ("RUST_LOG".into(), "debug".into()),
        ("ANTHROPIC_API_KEY".into(), "must-not-arrive".into()),
    ];

    let seen = run(&backend, command).await;

    assert!(
        seen.contains("LOG=debug"),
        "an allowlisted variable did not reach the held container: {seen}"
    );
    assert!(
        seen.contains("SECRET=unset"),
        "a credential reached the held container: {seen}"
    );
}
