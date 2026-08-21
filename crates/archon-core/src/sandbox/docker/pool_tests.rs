//! What the key is made of, and what the held container's arguments say.
//!
//! The container itself is proved in `tests/sandbox_docker_world.rs` against a
//! real daemon; nothing here can establish that a container was reused, and
//! nothing here pretends to.

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
    let labels = vec![
        (OWNED_LABEL.to_string(), "1".to_string()),
        (OWNER_LABEL.to_string(), "abc123".to_string()),
        (PID_LABEL.to_string(), "4242".to_string()),
    ];

    let args = docker_pool_create_args(
        &config(),
        "rw",
        std::path::Path::new("/repo"),
        "archon-sbx-abc123-0",
        &labels,
        900,
    );

    for expected in [
        "archon.sandbox=1",
        "archon.sandbox.owner=abc123",
        "archon.sandbox.pid=4242",
    ] {
        assert!(
            args.contains(&expected.to_string()),
            "missing {expected} in {args:?}"
        );
    }
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

/// The bound is what stops a leaked container living forever, so a value that
/// would kill commands mid-flight is a configuration error, not a preference.
#[test]
fn an_absurdly_short_container_lifetime_is_refused_at_config_load() {
    assert!(max_age_is_sane(30).is_err());
    assert!(max_age_is_sane(60).is_ok());
    assert!(max_age_is_sane(DEFAULT_MAX_AGE_SECS).is_ok());
}
