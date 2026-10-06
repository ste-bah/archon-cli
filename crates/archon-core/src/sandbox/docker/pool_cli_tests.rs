//! What the pool does when the daemon answers, refuses, or never answers.
//! Every daemon here is a fake; no test runs the real `docker`.

use super::super::fake_docker::{FakeDocker, TEST_BOUND, WELL_UNDER_A_HANG};
use super::*;
use std::time::Instant;

fn pool_on(fake: &FakeDocker, scope: SandboxScope) -> ContainerPool {
    let config = DockerConfig {
        enabled: true,
        binary: fake.binary(),
        ..DockerConfig::default()
    };
    ContainerPool::new(config, "rw".into(), scope).with_cli_bound(TEST_BOUND)
}

fn request(session: &str, turn: &str) -> SandboxCommandRequest {
    SandboxCommandRequest {
        command: "true".into(),
        working_dir: PathBuf::from("/repo"),
        timeout_ms: 1_000,
        max_output_bytes: 1024,
        env: Vec::new(),
        session_id: session.into(),
        turn_id: Some(turn.into()),
    }
}

/// `ps` (reaping) lists nothing; each other subcommand does what `rest` says.
fn daemon(run: &str, inspect: &str, rm: &str) -> FakeDocker {
    FakeDocker::new(&format!(
        "case \"$1\" in\n  ps) exit 0 ;;\n  run) {run} ;;\n  inspect) {inspect} ;;\n  rm) {rm} ;;\nesac"
    ))
}

const HANG: &str = "sleep 30";
const OK: &str = "exit 0";

fn calls_to(fake: &FakeDocker, subcommand: &str) -> Vec<String> {
    fake.calls()
        .into_iter()
        .filter(|call| call.split(' ').next() == Some(subcommand))
        .collect()
}

#[tokio::test]
async fn an_answered_create_holds_a_container_and_drop_removes_it() {
    let fake = daemon("echo container-id", OK, OK);
    let pool = pool_on(&fake, SandboxScope::Session);

    let lease = pool
        .container_for(&request("s1", "s1#1"))
        .await
        .expect("answered")
        .expect("session scope holds a container");
    let name = lease.name().to_string();
    drop(lease);
    drop(pool);

    assert!(name.starts_with("archon-sbx-"), "{name}");
    assert_eq!(calls_to(&fake, "run").len(), 1);
    assert_eq!(calls_to(&fake, "rm"), vec![format!("rm --force {name}")]);
}

/// The turn boundary used to wait on `docker run` for ever. Now it refuses,
/// naming the call and the daemon state.
#[tokio::test]
async fn a_create_the_daemon_never_answers_is_refused_within_the_bound() {
    let fake = daemon(HANG, OK, OK);
    let pool = pool_on(&fake, SandboxScope::Turn);
    let started = Instant::now();

    let error = pool
        .container_for(&request("s1", "s1#1"))
        .await
        .err()
        .expect("no container from a daemon that never answered");

    assert!(
        started.elapsed() < WELL_UNDER_A_HANG,
        "the bound did not fire"
    );
    assert!(error.is_no_answer(), "{error:?}");
    assert!(error.to_string().contains("run --detach"), "{error}");
    assert!(
        pool.live.lock().await.is_empty(),
        "a container that may not exist must not be held"
    );
    // Killing the CLI does not stop the daemon, so the container may still
    // appear. Teardown must know its name.
    let unconfirmed = pool.unconfirmed.lock().await.clone();
    assert_eq!(unconfirmed.len(), 1, "{unconfirmed:?}");
    drop(pool);
    assert_eq!(
        calls_to(&fake, "rm"),
        vec![format!("rm --force {}", unconfirmed[0])]
    );
}

/// Once the daemon answers again, the container it never confirmed is removed.
#[tokio::test]
async fn an_unanswered_create_is_cleaned_up_once_the_daemon_answers_again() {
    let fake = daemon(
        "if [ -f \"$FAKE_DIR/hung-once\" ]; then echo id; else touch \"$FAKE_DIR/hung-once\"; sleep 30; fi",
        OK,
        OK,
    );
    let pool = pool_on(&fake, SandboxScope::Session);
    let req = request("s1", "s1#1");
    assert!(
        pool.container_for(&req).await.is_err(),
        "first create hangs"
    );
    let lost = pool.unconfirmed.lock().await[0].clone();

    let lease = pool
        .container_for(&req)
        .await
        .expect("answered")
        .expect("held");

    assert_ne!(lease.name(), lost);
    assert_eq!(calls_to(&fake, "rm"), vec![format!("rm --force {lost}")]);
    assert!(pool.unconfirmed.lock().await.is_empty());
}

/// Reaping is the first call a pool makes. If the daemon does not answer it,
/// the command is refused there rather than put to the same daemon again.
#[tokio::test]
async fn a_reap_the_daemon_never_answers_refuses_without_another_call() {
    let fake = FakeDocker::new("case \"$1\" in ps) sleep 30 ;; *) exit 0 ;; esac");
    let pool = pool_on(&fake, SandboxScope::Tool);

    let error = pool
        .container_for(&request("s1", "s1#1"))
        .await
        .err()
        .expect("refused");

    assert!(error.is_no_answer(), "{error:?}");
    assert_eq!(fake.calls().len(), 1, "{:?}", fake.calls());
}

#[tokio::test]
async fn a_create_the_daemon_refuses_is_a_failure_with_the_daemons_reason() {
    let fake = daemon("echo 'Unable to find image' >&2; exit 125", OK, OK);
    let pool = pool_on(&fake, SandboxScope::Session);

    let error = pool
        .container_for(&request("s1", "s1#1"))
        .await
        .err()
        .expect("refused");

    let DockerCliError::Failed { stderr, .. } = &error else {
        panic!("wrong error: {error:?}");
    };
    assert!(stderr.contains("Unable to find image"), "{stderr}");
}

/// No answer is not "gone". Concluding that it was would forget a container
/// that may still be running and start a second one beside it.
#[tokio::test]
async fn a_container_whose_state_cannot_be_asked_is_not_forgotten() {
    let fake = daemon("echo container-id", HANG, OK);
    let pool = pool_on(&fake, SandboxScope::Session);
    let req = request("s1", "s1#1");
    let name = pool
        .container_for(&req)
        .await
        .expect("answered")
        .expect("held")
        .name()
        .to_string();
    let started = Instant::now();

    let error = pool
        .forget_if_gone(&req, &name)
        .await
        .expect_err("the daemon was never asked successfully");

    assert!(
        started.elapsed() < WELL_UNDER_A_HANG,
        "the bound did not fire"
    );
    assert!(error.is_no_answer(), "{error:?}");
    let again = pool.container_for(&req).await.expect("held").expect("held");
    assert_eq!(
        again.name(),
        name,
        "the container was forgotten on no answer"
    );
}

/// A failed `inspect` is the daemon's answer that it knows no such container.
#[tokio::test]
async fn a_container_the_daemon_does_not_know_is_forgotten() {
    let fake = daemon("echo container-id", "exit 1", OK);
    let pool = pool_on(&fake, SandboxScope::Session);
    let req = request("s1", "s1#1");
    let name = pool
        .container_for(&req)
        .await
        .expect("answered")
        .expect("held")
        .name()
        .to_string();

    assert_eq!(pool.forget_if_gone(&req, &name).await, Ok(true));
    assert!(pool.live.lock().await.is_empty());
}

/// A teardown the daemon never answers must not hold the next turn hostage,
/// nor be followed by a create that would wait out a second bound.
#[tokio::test]
async fn a_turn_boundary_whose_teardown_gets_no_answer_refuses_within_one_bound() {
    let fake = daemon("echo container-id", OK, HANG);
    let pool = pool_on(&fake, SandboxScope::Turn);
    pool.container_for(&request("s1", "s1#1"))
        .await
        .expect("answered")
        .expect("held");
    let started = Instant::now();

    let error = pool
        .container_for(&request("s1", "s1#2"))
        .await
        .err()
        .expect("refused: the daemon stopped answering");

    assert!(error.is_no_answer(), "{error:?}");
    assert!(
        started.elapsed() < TEST_BOUND * 2,
        "more than one bound was spent: {:?}",
        started.elapsed()
    );
    assert_eq!(
        calls_to(&fake, "rm").len(),
        1,
        "the old turn's teardown ran"
    );
    assert_eq!(calls_to(&fake, "run").len(), 1, "no create after the stall");
    let old = pool.unconfirmed.lock().await.clone();
    assert_eq!(old.len(), 1, "the unremoved container is still tracked");
    // `Drop` meets the same hung daemon and must be bounded too.
    let dropping = Instant::now();
    drop(pool);
    assert!(dropping.elapsed() < WELL_UNDER_A_HANG, "Drop hung on rm");
    assert_eq!(
        calls_to(&fake, "rm").last(),
        Some(&format!("rm --force {}", old[0]))
    );
}
