//! What the key is made of, and what the held container's arguments say.
//!
//! The container itself is proved in `tests/sandbox_docker_world.rs` against a
//! real daemon; nothing here can establish that a container was reused, and
//! nothing here pretends to.

use super::super::exec::{docker_run_args, docker_terminal_args};
use super::*;
use std::path::PathBuf;

fn config() -> DockerConfig {
    DockerConfig {
        enabled: true,
        env_allowlist: vec!["ALLOWED".into(), "ANTHROPIC_API_KEY".into()],
        ..DockerConfig::default()
    }
}

fn pool(scope: SandboxScope) -> ContainerPool {
    ContainerPool::new(config(), "rw".into(), scope)
}

fn request(working_dir: &str, session: &str, turn: Option<&str>) -> SandboxCommandRequest {
    SandboxCommandRequest {
        command: "true".into(),
        working_dir: PathBuf::from(working_dir),
        timeout_ms: 1_000,
        max_output_bytes: 1024,
        env: vec![
            ("ALLOWED".into(), "yes".into()),
            ("ANTHROPIC_API_KEY".into(), "secret".into()),
        ],
        session_id: session.into(),
        turn_id: turn.map(ToOwned::to_owned),
    }
}

#[test]
fn tool_scope_holds_nothing() {
    assert!(
        pool(SandboxScope::Tool)
            .key(&request("/repo", "s1", Some("s1#1")))
            .is_none(),
        "`tool` means a container per command; holding one would be the opposite"
    );
}

/// The whole reason the key is not just the scope instance. A worktree-isolated
/// subagent mounts a different tree and inherits its parent's turn id; sharing a
/// container across the two would put both agents in one world while each
/// believed it was isolated.
#[test]
fn two_working_directories_never_share_a_lifetime() {
    let pool = pool(SandboxScope::Session);

    let parent = pool
        .key(&request("/repo", "s1", Some("s1#1")))
        .expect("held");
    let child = pool
        .key(&request("/repo-worktree", "s1", Some("s1#1")))
        .expect("held");

    assert_ne!(parent, child);
}

/// Under `session` scope the session *is* the instance, so this would hold
/// anyway. The case that needs the field is `turn`: turn ids are minted by their
/// agent, and two sessions in one process must not be able to collide into one
/// container by both naming a turn the same way.
#[test]
fn two_sessions_never_share_a_lifetime_even_on_a_colliding_turn_id() {
    let pool = pool(SandboxScope::Turn);

    assert_ne!(
        pool.key(&request("/repo", "s1", Some("turn-1")))
            .expect("held"),
        pool.key(&request("/repo", "s2", Some("turn-1")))
            .expect("held")
    );
}

#[test]
fn session_scope_ignores_the_turn_and_turn_scope_does_not() {
    let session = pool(SandboxScope::Session);
    assert_eq!(
        session.key(&request("/repo", "s1", Some("s1#1"))),
        session.key(&request("/repo", "s1", Some("s1#2"))),
        "a session outlives its turns"
    );

    let turn = pool(SandboxScope::Turn);
    assert_ne!(
        turn.key(&request("/repo", "s1", Some("s1#1"))),
        turn.key(&request("/repo", "s1", Some("s1#2"))),
        "`turn` that survived a turn boundary is `session` under another name"
    );
}

/// `None` is not an identity. Two unrelated callers that both cannot name their
/// turn have nothing in common, and collapsing them into one container would be
/// exactly the cross-agent leak the working directory is in the key to prevent.
#[test]
fn a_turn_scoped_request_that_cannot_name_its_turn_holds_nothing() {
    assert!(
        pool(SandboxScope::Turn)
            .key(&request("/repo", "s1", None))
            .is_none()
    );
}

/// Turns are sequential *within* a session, and only within one. A concurrent
/// session's turns are not ordered against this one's, so evicting on a turn-id
/// mismatch alone would tear down another session's container — very possibly
/// with a command running in it.
#[test]
fn a_new_turn_ends_only_its_own_sessions_earlier_turns() {
    let pool = pool(SandboxScope::Turn);
    let mine_old = pool
        .key(&request("/repo", "s1", Some("s1#1")))
        .expect("held");
    let mine_new = pool
        .key(&request("/repo", "s1", Some("s1#2")))
        .expect("held");
    let theirs = pool
        .key(&request("/repo", "s2", Some("s2#1")))
        .expect("held");
    let mine_elsewhere = pool
        .key(&request("/worktree", "s1", Some("s1#1")))
        .expect("held");

    let ended = finished_turns(
        [&mine_old, &mine_new, &theirs, &mine_elsewhere].into_iter(),
        &mine_new,
    );

    assert!(ended.contains(&mine_old), "the previous turn must end");
    assert!(
        ended.contains(&mine_elsewhere),
        "the previous turn's worktree container is part of the same turn"
    );
    assert!(
        !ended.contains(&theirs),
        "a concurrent session's container was torn down by this session's turn boundary"
    );
    assert!(!ended.contains(&mine_new), "the current turn must survive");
}

/// Teardown's only handle on a container whose creator is gone.
#[test]
fn create_arguments_carry_the_labels_teardown_finds_containers_by() {
    let args = docker_pool_create_args(
        &config(),
        "rw",
        std::path::Path::new("/repo"),
        "archon-sbx-abc123-0",
        900,
    );

    assert_labelled(&args, "held");
    assert!(args.contains(&"--detach".to_string()));
    assert!(
        args.contains(&"--rm".to_string()),
        "without --rm the container that outlives its sleep stays as a husk"
    );
    assert_eq!(
        args.iter().rev().take(2).collect::<Vec<_>>(),
        vec!["900", "sleep"],
        "the container's own lifetime bound is its PID 1"
    );
}

/// **Every** container, not just the pooled ones.
///
/// The labels started life on the pool's containers alone, which left the
/// per-command and terminal containers — the two nothing else ever tears down —
/// invisible to `docker ps --filter label=archon.sandbox=1` and therefore
/// uncollectable by reaping. A timed-out per-command container leaked with
/// nothing able to find it, and a terminal whose owner was killed leaked
/// forever.
#[test]
fn every_container_archon_starts_is_labelled_and_therefore_findable() {
    let per_command = docker_run_args(&config(), "rw", &request("/repo", "s1", None));
    assert_labelled(&per_command, "command");

    let terminal = docker_terminal_args(
        &config(),
        "rw",
        std::path::Path::new("/repo"),
        "/workspace",
        "/bin/bash",
    );
    assert_labelled(&terminal, "terminal");
}

/// A terminal's container is the one most likely to outlive its owner, and it
/// was the only one with no age bound at all.
#[test]
fn a_terminal_container_carries_the_same_age_bound_as_a_held_one() {
    let cfg = DockerConfig {
        container_max_age_secs: 900,
        ..config()
    };

    let args = docker_terminal_args(
        &cfg,
        "rw",
        std::path::Path::new("/repo"),
        "/workspace",
        "/bin/bash",
    );

    assert_eq!(
        args.iter().rev().take(4).collect::<Vec<_>>(),
        vec!["/bin/bash", "900", "--signal=KILL", "timeout"],
        "the shell must run under an age bound, and the signal must be KILL: \
         measured, an interactive bash ignores the SIGTERM plain `timeout` sends"
    );
}

fn assert_labelled(args: &[String], kind: &str) {
    for expected in [
        format!("{OWNED_LABEL}=1"),
        format!("{OWNER_LABEL}={}", owner_id()),
        format!("{PID_LABEL}={}", std::process::id()),
        format!("archon.sandbox.kind={kind}"),
    ] {
        assert!(
            args.contains(&expected),
            "missing label {expected} — a container Archon cannot find is one it \
             cannot clean up: {args:?}"
        );
    }
}

/// A held container must not become a way to smuggle in a variable the
/// per-command path would have dropped.
#[test]
fn a_held_containers_command_carries_only_allowlisted_non_secret_environment() {
    let args = docker_exec_args(
        &config(),
        "archon-sbx-abc123-0",
        &request("/repo", "s1", Some("s1#1")),
    );

    assert!(args.contains(&"ALLOWED=yes".to_string()));
    assert!(
        !args.iter().any(|arg| arg.contains("secret")),
        "a credential reached the container: {args:?}"
    );
    assert_eq!(args[0], "exec");
    assert_eq!(
        args.iter().rev().take(3).collect::<Vec<_>>(),
        vec!["true", "-lc", "/bin/bash"]
    );
}

/// A busy container must survive the turn boundary.
///
/// `docker rm --force` on a container with a command inside it kills that
/// command, and the model gets a bare `Exit code 137` for a container Archon
/// destroyed itself. "Turns are sequential" holds for one agent's own turns and
/// nothing enforces it across a tree — a subagent inherits its parent's turn id
/// and can still be running when the parent's next turn starts.
#[tokio::test]
async fn a_container_with_a_command_still_in_it_survives_the_turn_boundary() {
    let pool = pool(SandboxScope::Turn);
    let previous = pool
        .key(&request("/repo", "s1", Some("s1#1")))
        .expect("held");
    let current = pool
        .key(&request("/repo", "s1", Some("s1#2")))
        .expect("held");
    let mut live = HashMap::new();
    live.insert(previous.clone(), held("archon-sbx-busy"));
    let _lease = live[&previous].lease();

    pool.evict_finished_turns(&mut live, &current).await;

    assert!(
        live.contains_key(&previous),
        "the previous turn's container was torn down with a command still \
         running in it"
    );
}

/// And the deferral must be a deferral, not an exemption: once the command
/// finishes, the next turn boundary collects it.
#[tokio::test]
async fn the_same_container_is_collected_once_its_command_finishes() {
    let pool = pool(SandboxScope::Turn);
    let previous = pool
        .key(&request("/repo", "s1", Some("s1#1")))
        .expect("held");
    let current = pool
        .key(&request("/repo", "s1", Some("s1#2")))
        .expect("held");
    let mut live = HashMap::new();
    live.insert(previous.clone(), held("archon-sbx-idle"));

    let lease = live[&previous].lease();
    pool.evict_finished_turns(&mut live, &current).await;
    assert!(live.contains_key(&previous), "busy, so deferred");

    drop(lease);
    pool.evict_finished_turns(&mut live, &current).await;

    assert!(
        !live.contains_key(&previous),
        "an idle container from a finished turn must not survive a second boundary"
    );
}

/// A lease is released even when the command it covers panics or is cancelled,
/// because the count is a `Drop` and not a manual decrement.
#[test]
fn a_lease_is_released_when_its_command_unwinds() {
    let container = held("archon-sbx-panicky");
    let counter = std::sync::Arc::clone(&container.in_flight);

    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _lease = container.lease();
        assert_eq!(counter.load(std::sync::atomic::Ordering::SeqCst), 1);
        panic!("the command blew up");
    }));

    assert!(unwound.is_err(), "the test's own premise");
    assert_eq!(
        counter.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "a container stays pinned forever if a lease can be lost to a panic"
    );
}

fn held(name: &str) -> Held {
    Held {
        name: name.into(),
        in_flight: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
    }
}

/// The bound is what stops a leaked container living forever, so a value that
/// would kill commands mid-flight is a configuration error, not a preference.
#[test]
fn an_absurdly_short_container_lifetime_is_refused_at_config_load() {
    assert!(max_age_is_sane(30).is_err());
    assert!(max_age_is_sane(60).is_ok());
    assert!(max_age_is_sane(DEFAULT_MAX_AGE_SECS).is_ok());
}
