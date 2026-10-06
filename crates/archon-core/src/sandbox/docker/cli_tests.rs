//! The no-progress bound, proved against fake docker binaries. No test here
//! runs the real `docker`.
#![cfg(unix)]

use super::super::fake_docker::{FakeDocker, TEST_BOUND, WELL_UNDER_A_HANG};
use super::*;

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|arg| (*arg).to_string()).collect()
}

/// The incident: a daemon that never answers. The call must end within the
/// bound, say which call it was, and leave nothing it started running.
#[tokio::test]
async fn a_call_the_daemon_never_answers_is_killed_with_its_group_and_named() {
    let fake = FakeDocker::new("sleep 30 & echo $! > \"$FAKE_DIR/child.pid\"\nwait");
    let started = Instant::now();

    let error = run(
        &fake.binary(),
        &args(&["rm", "--force", "archon-sbx-idle"]),
        "docker rm --force archon-sbx-idle",
        TEST_BOUND,
    )
    .await
    .expect_err("a call with no answer must not succeed");

    assert!(
        started.elapsed() < WELL_UNDER_A_HANG,
        "the bound did not fire"
    );
    assert!(error.is_no_answer(), "wrong error: {error:?}");
    let message = error.to_string();
    assert!(
        message.contains("docker rm --force archon-sbx-idle"),
        "{message}"
    );
    assert!(message.contains("daemon state: not answering"), "{message}");
    assert_grandchild_is_gone(&fake);
}

/// A bound on totals would kill a slow call that is still making progress.
/// Output restarts the deadline, so a call that runs for several bounds while
/// writing is left to finish.
#[tokio::test]
async fn a_call_that_keeps_writing_outlives_the_bound() {
    let fake = FakeDocker::new(
        "i=0\nwhile [ $i -lt 12 ]; do echo tick >&2; sleep 0.25; i=$((i+1)); done\necho done",
    );
    let started = Instant::now();

    let output = run(&fake.binary(), &args(&["ps"]), "docker ps", TEST_BOUND)
        .await
        .expect("a call making progress must not be killed");

    assert!(
        started.elapsed() > TEST_BOUND,
        "the premise: the call ran longer than one bound"
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "done");
}

#[tokio::test]
async fn an_answered_call_returns_its_output() {
    let fake = FakeDocker::new("echo true");

    let output = run(
        &fake.binary(),
        &args(&["inspect", "-f", "{{.State.Running}}", "c1"]),
        "docker inspect c1",
        TEST_BOUND,
    )
    .await
    .expect("answered");

    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "true");
    assert_eq!(fake.calls(), vec!["inspect -f {{.State.Running}} c1"]);
}

#[tokio::test]
async fn a_refusal_from_the_daemon_is_a_failure_with_its_reason() {
    let fake = FakeDocker::new("echo 'Error response from daemon: No such container' >&2\nexit 3");

    let error = run(
        &fake.binary(),
        &args(&["rm", "--force", "c1"]),
        "docker rm --force c1",
        TEST_BOUND,
    )
    .await
    .expect_err("non-zero exit");

    let DockerCliError::Failed {
        call,
        status,
        stderr,
    } = error
    else {
        panic!("wrong error: {error:?}");
    };
    assert_eq!(call, "docker rm --force c1");
    assert!(status.contains('3'), "{status}");
    assert!(stderr.contains("No such container"), "{stderr}");
}

#[tokio::test]
async fn a_binary_that_cannot_start_says_so() {
    let error = run(
        "/nonexistent/archon-fake-docker",
        &args(&["ps"]),
        "docker ps",
        TEST_BOUND,
    )
    .await
    .expect_err("missing binary");

    assert!(matches!(error, DockerCliError::Spawn { .. }), "{error:?}");
}

/// The pool's `Drop` cannot await, and used to block on `status()` for ever.
#[test]
fn the_blocking_call_is_bounded_too() {
    let fake = FakeDocker::new("sleep 30 & echo $! > \"$FAKE_DIR/child.pid\"\nwait");
    let started = Instant::now();

    let error = run_blocking(
        &fake.binary(),
        &args(&["rm", "--force", "a", "b"]),
        "docker rm --force a b",
        TEST_BOUND,
    )
    .expect_err("no answer");

    assert!(
        started.elapsed() < WELL_UNDER_A_HANG,
        "the bound did not fire"
    );
    assert!(error.is_no_answer(), "{error:?}");
    assert!(error.to_string().contains("docker rm --force a b"));
    assert_grandchild_is_gone(&fake);
}

#[test]
fn the_blocking_call_reports_success_and_failure() {
    let ok = FakeDocker::new("exit 0");
    assert_eq!(
        run_blocking(&ok.binary(), &[], "docker rm", TEST_BOUND),
        Ok(())
    );

    let failing = FakeDocker::new("exit 1");
    let error = run_blocking(&failing.binary(), &[], "docker rm", TEST_BOUND).expect_err("exit 1");
    assert!(matches!(error, DockerCliError::Failed { .. }), "{error:?}");
}

/// Killing only the docker client would leave whatever it started holding the
/// pipes and the daemon connection. The whole group goes.
fn assert_grandchild_is_gone(fake: &FakeDocker) {
    let pid: u32 = std::fs::read_to_string(fake.dir().join("child.pid"))
        .expect("the fake recorded its child")
        .trim()
        .parse()
        .expect("pid");
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut system = sysinfo::System::new();
    // A killed orphan may linger as a zombie until its new parent reaps it;
    // `owner_is_alive` counts a zombie as dead.
    while super::super::reap::owner_is_alive(&mut system, Some(pid)) {
        assert!(
            Instant::now() < deadline,
            "the hung call's child {pid} survived the kill"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}
