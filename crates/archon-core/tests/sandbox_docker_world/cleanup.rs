//! Teardown: the half where the guarantee `--rm` used to give has to be rebuilt.
//!
//! Holding a container open means Archon now owns its destruction. These prove
//! the two mechanisms that can be exercised from a test process — the scope
//! boundary and startup reaping — against a real daemon. The third,
//! `container_max_age_secs`, is the container's own PID 1 and needs no host
//! involvement at all; it is set to two minutes here so anything that escapes a
//! panicking test dies on its own.

use super::*;

/// The terminal container's own labels and age bound. A sibling file only
/// because of the 500-line ceiling; it uses the helpers here.
#[path = "terminal_lifetime.rs"]
mod terminal_lifetime;

/// Reaping used to sit *below* the `key()` bail-out in `container_for`, so under
/// `scope = "tool"` — where the key is always `None` — it never ran at all.
///
/// The configuration that creates the most uncollectable containers, one per
/// command, was the one that collected none of them. The same held for a
/// `turn`-scoped caller with no turn id, which the TUI's pipeline adapter is.
#[tokio::test]
#[ignore = "requires a Docker daemon and the ubuntu:24.04 image"]
async fn startup_reaping_runs_even_when_no_container_is_ever_held() {
    for (scope, turn) in [
        (SandboxScope::Tool, Some("t#1")),
        (SandboxScope::Turn, None),
    ] {
        let dir = tempfile::tempdir().expect("tempdir");
        let suffix = uuid::Uuid::new_v4().simple().to_string()[..8].to_string();
        let orphaned = format!("archon-reap-nokey-{suffix}");
        let _cleanup = Removed(orphaned.clone());
        plant(&orphaned, "a-dead-archon", 0);

        let backend = backend(scope);
        let mut command = request(dir.path(), &suffix, "unused", "true");
        command.turn_id = turn.map(ToOwned::to_owned);
        run(&backend, command).await;

        assert!(
            !container_exists(&orphaned),
            "{scope} scope holds no container, and so never reaped one either — \
             leaving every orphan from a dead Archon running"
        );
    }
}

/// Every container Archon starts has to be findable, not just the pooled ones.
///
/// `docker ps --filter label=archon.sandbox=1` is the command the docs hand
/// operators. A per-command container carried no labels at all, so that command
/// could never show it and reaping could never collect it — and a per-command
/// container is exactly what a timed-out command leaves behind.
#[tokio::test]
#[ignore = "requires a Docker daemon and the ubuntu:24.04 image"]
async fn a_per_command_container_is_findable_by_label_while_it_runs() {
    let dir = tempfile::tempdir().expect("tempdir");
    let backend = backend(SandboxScope::Tool);

    let running = tokio::spawn({
        let workspace = dir.path().to_path_buf();
        async move {
            backend
                .execute_bash(request(&workspace, "labelled", "labelled#1", "sleep 12"))
                .await
        }
    });

    let found = wait_for_labelled_container("command", dir.path()).await;
    running.abort();

    assert!(
        found,
        "the per-command container is invisible to the label filter, so nothing \
         Archon has can find it once the command that owns it is gone"
    );
}

/// Whether a container of `kind` is mounted on `workspace` right now.
///
/// Matched on the mount source rather than only the label, so a sibling test's
/// container cannot satisfy this one.
async fn wait_for_labelled_container(kind: &str, workspace: &Path) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while std::time::Instant::now() < deadline {
        for id in labelled_container_ids(kind) {
            if mount_sources(&id).iter().any(|source| {
                std::path::Path::new(source) == workspace
                    // Docker resolves symlinks in bind sources, and macOS and
                    // some CI images put temp dirs behind one.
                    || std::fs::canonicalize(source).ok().as_deref()
                        == std::fs::canonicalize(workspace).ok().as_deref()
            }) {
                return true;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    false
}

fn labelled_container_ids(kind: &str) -> Vec<String> {
    let output = std::process::Command::new("docker")
        .args([
            "ps",
            "--quiet",
            "--no-trunc",
            "--filter",
            "label=archon.sandbox=1",
            "--filter",
            &format!("label=archon.sandbox.kind={kind}"),
        ])
        .output();
    output
        .ok()
        .filter(|output| output.status.success())
        .map(|output| {
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(|line| line.trim().to_string())
                .filter(|line| !line.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn mount_sources(id: &str) -> Vec<String> {
    std::process::Command::new("docker")
        .args(["inspect", "-f", "{{range .Mounts}}{{.Source}}\n{{end}}", id])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| {
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(|line| line.trim().to_string())
                .filter(|line| !line.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

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

/// A container that goes away *while a command is inside it* must say so.
///
/// The in-flight guard means Archon no longer does this to itself at a turn
/// boundary, but a container can still vanish under a running command — its age
/// bound expires, an operator runs `docker rm`, the daemon restarts. `docker
/// exec` reports that as `Exit code 137`, which reads as a memory limit and
/// sends the model looking in entirely the wrong place.
///
/// The command is *not* re-run: it already ran, and repeating it would repeat
/// whatever side effects it got through before it was killed.
#[tokio::test]
#[ignore = "requires a Docker daemon and the ubuntu:24.04 image"]
async fn a_command_killed_by_its_container_disappearing_is_told_so() {
    let dir = tempfile::tempdir().expect("tempdir");
    let backend = std::sync::Arc::new(backend(SandboxScope::Session));

    // One command to establish the container, so the second can be removed out
    // from under a command that is already running in it.
    let established = run(
        &backend,
        request(dir.path(), "killed", "killed#1", WHICH_CONTAINER),
    )
    .await;
    let id = container_id(&established);
    let _cleanup = Removed(id.clone());

    let long_running = tokio::spawn({
        let backend = std::sync::Arc::clone(&backend);
        let workspace = dir.path().to_path_buf();
        async move {
            backend
                .execute_bash(request(
                    &workspace,
                    "killed",
                    "killed#1",
                    "printf started; sleep 30",
                ))
                .await
                .expect("the docker backend executes bash")
        }
    });

    // Removed once the command is demonstrably inside the container, not on a
    // fixed sleep: `docker top` is what says a process of ours is running.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while std::time::Instant::now() < deadline && !container_runs_a_sleep(&id) {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    assert!(
        container_runs_a_sleep(&id),
        "the command never got inside the container, so this proves nothing"
    );
    remove_container(&id);

    let result = long_running.await.expect("no panic");

    assert!(result.is_error, "a killed command is not a success");
    assert!(
        result
            .content
            .contains("stopped before the command finished"),
        "a bare exit code tells the model nothing it can act on: {}",
        result.content
    );
    assert_eq!(
        result.content.matches("started").count(),
        1,
        "the command was re-run after being killed; whatever side effects it had \
         already had would happen twice: {}",
        result.content
    );
}

fn container_runs_a_sleep(id: &str) -> bool {
    std::process::Command::new("docker")
        .args(["top", id])
        .output()
        .is_ok_and(|output| {
            output.status.success() && String::from_utf8_lossy(&output.stdout).contains("sleep 30")
        })
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
