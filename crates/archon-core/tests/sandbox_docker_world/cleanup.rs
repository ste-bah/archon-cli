//! Teardown: the half where the guarantee `--rm` used to give has to be rebuilt.
//!
//! Holding a container open means Archon now owns its destruction. These prove
//! the two mechanisms that can be exercised from a test process — the scope
//! boundary and startup reaping — against a real daemon. The third,
//! `container_max_age_secs`, is the container's own PID 1 and needs no host
//! involvement at all; it is set to two minutes here so anything that escapes a
//! panicking test dies on its own.

use super::*;

/// A container can go away for reasons that are nobody's bug: its own age bound
/// expires, an operator runs `docker rm`, the daemon restarts. The next command
/// must rebuild rather than hand the model a daemon error.
#[tokio::test]
#[ignore = "requires a Docker daemon and the ubuntu:24.04 image"]
async fn a_held_container_that_disappears_underneath_us_is_rebuilt() {
    let dir = tempfile::tempdir().expect("tempdir");
    let backend = backend(SandboxScope::Session);

    let first = run(
        &backend,
        request(dir.path(), "vanish", "vanish#1", WHICH_CONTAINER),
    )
    .await;
    let first_id = container_id(&first);

    remove_container(&first_id);
    assert!(!container_exists(&first_id), "the fixture did not take");

    let second = run(
        &backend,
        request(dir.path(), "vanish", "vanish#2", WHICH_CONTAINER),
    )
    .await;

    assert_ne!(first_id, container_id(&second));
}

/// The session boundary. Best-effort by construction — `Drop` does not run under
/// SIGKILL — which is why it is one of three mechanisms, but it is the one that
/// runs on every ordinary exit and it has to actually work.
#[tokio::test]
#[ignore = "requires a Docker daemon and the ubuntu:24.04 image"]
async fn dropping_the_backend_removes_the_container_it_was_holding() {
    let dir = tempfile::tempdir().expect("tempdir");

    let id = {
        let backend = backend(SandboxScope::Session);
        let output = run(
            &backend,
            request(dir.path(), "dropped", "dropped#1", WHICH_CONTAINER),
        )
        .await;
        let id = container_id(&output);
        assert!(container_exists(&id), "nothing was held to tear down");
        id
    };
    let _cleanup = Removed(id.clone());

    assert!(
        !container_exists(&id),
        "the session's container outlived the session that held it"
    );
}

/// Reaping decides whether to destroy a container this process did not create,
/// so both halves of that decision are proved against the daemon.
///
/// The live-owner fixture is the important one. Parallel Archon sessions on one
/// machine are ordinary here, and reaping on "not mine" alone would have two
/// runs killing each other's containers mid-command.
#[tokio::test]
#[ignore = "requires a Docker daemon and the ubuntu:24.04 image"]
async fn startup_reaping_removes_a_dead_owners_container_and_spares_a_live_ones() {
    let dir = tempfile::tempdir().expect("tempdir");
    let suffix = uuid::Uuid::new_v4().simple().to_string()[..8].to_string();
    let orphaned = format!("archon-reap-orphan-{suffix}");
    let live = format!("archon-reap-live-{suffix}");
    let _cleanup = (Removed(orphaned.clone()), Removed(live.clone()));

    // Pid 0 names no process on any platform, so this is a container whose
    // owner is definitively gone — not merely one whose pid looks unlikely.
    plant(&orphaned, "a-dead-archon", 0);
    plant(&live, "another-live-archon", std::process::id());
    // The orphan is deliberately not asserted present here: a sibling test in
    // this binary may already have reaped it through its own pool, which is the
    // code under test doing exactly its job. `plant` asserts the container
    // started, which is what "the fixture landed" means. The live fixture has
    // no such race — nothing may remove it, and that is the claim.
    assert!(container_exists(&live), "the fixture did not take");

    // Reaping is not public API: it happens once per process, before the first
    // container is created. Running a command is how a real session triggers it.
    let backend = backend(SandboxScope::Session);
    run(&backend, request(dir.path(), "reaper", "reaper#1", "true")).await;

    assert!(
        !container_exists(&orphaned),
        "a container whose creating process is gone was left running, holding \
         its memory and its mounts"
    );
    assert!(
        container_exists(&live),
        "reaping destroyed a container belonging to a *running* Archon; two \
         parallel sessions would tear each other down"
    );
}

/// A container carrying Archon's labels, created outside the pool so the test
/// controls whose it claims to be.
fn plant(name: &str, owner: &str, pid: u32) {
    let status = std::process::Command::new("docker")
        .args([
            "run",
            "--detach",
            "--rm",
            "--pull",
            "never",
            "--name",
            name,
            "--label",
            "archon.sandbox=1",
            "--label",
            &format!("archon.sandbox.owner={owner}"),
            "--label",
            &format!("archon.sandbox.pid={pid}"),
            "--network",
            "none",
            "ubuntu:24.04",
            "sleep",
            "120",
        ])
        .output()
        .expect("docker run");
    assert!(
        status.status.success(),
        "could not plant {name}: {}",
        String::from_utf8_lossy(&status.stderr)
    );
}
