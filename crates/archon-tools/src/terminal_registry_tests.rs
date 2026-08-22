//! Tests for the terminal registry (#189 Phase 6).
//!
//! These open real shells. The acceptance criteria for this phase are all about
//! what a live process does â€” state surviving between calls, output arriving
//! while nobody is listening, nothing left running afterwards â€” and none of
//! them can be shown against a fake.

use super::*;

fn shell_name() -> &'static str {
    // The fastest shell to start on each platform; what is being tested here is
    // the registry, not shell selection, which has its own tests.
    if cfg!(windows) { "cmd" } else { "sh" }
}

fn open(session_id: &str) -> Arc<Terminal> {
    let program = crate::terminal_shell::build(shell_name(), &std::env::temp_dir())
        .expect("the platform shell resolves");
    let id = format!("test-{}", uuid::Uuid::new_v4().simple());
    create(session_id, id, shell_name().to_string(), false, program).expect("terminal opens")
}

/// Poll until `needle` shows up, or give up. A shell answers when it answers.
async fn wait_for(terminal: &Terminal, needle: &str) -> String {
    for _ in 0..200 {
        let text = terminal.read(0, 1 << 20).text;
        if text.contains(needle) {
            return text;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    terminal.read(0, 1 << 20).text
}

#[tokio::test]
#[serial_test::serial]
async fn a_new_terminal_is_registered_and_findable() {
    let terminal = open("session-findable");

    assert!(get(&terminal.id).is_some());
    assert_eq!(terminal.shell, shell_name());

    close(&terminal.id);
    assert!(get(&terminal.id).is_none(), "close must deregister");
}

#[tokio::test]
#[serial_test::serial]
async fn closing_an_unknown_id_reports_it_rather_than_panicking() {
    assert!(!close("test-no-such-terminal"));
}

/// The headline acceptance criterion: `cd`, then a later command runs there.
/// This is the thing one-shot `Bash` cannot do at all.
#[tokio::test]
#[serial_test::serial]
async fn a_directory_change_holds_for_the_next_command() {
    let marker = format!("dir-{}", uuid::Uuid::new_v4().simple());
    let target = std::env::temp_dir().join(&marker);
    std::fs::create_dir_all(&target).expect("make the target directory");
    let terminal = open("session-cd");

    terminal.write(&format!("cd {}\n", target.display()));
    // `cd` with no argument prints the current directory on both shells.
    let show = if cfg!(windows) { "cd\n" } else { "pwd\n" };
    terminal.write(show);

    let seen = wait_for(&terminal, &marker).await;
    close(&terminal.id);
    let _ = std::fs::remove_dir_all(&target);

    assert!(
        seen.contains(&marker),
        "the second command did not run in the directory the first moved to: {seen:?}"
    );
}

/// The other headline criterion: start something long, return from the call,
/// and find the output waiting later.
#[tokio::test]
#[serial_test::serial]
async fn output_accumulates_while_nothing_is_reading() {
    let terminal = open("session-longrunning");
    let delayed = if cfg!(windows) {
        "ping -n 4 127.0.0.1 >nul & echo late-output\n"
    } else {
        "sleep 3; echo late-output\n"
    };
    terminal.write(delayed);

    // Immediately: the command is still running, so its result cannot be here.
    let straight_away = terminal.read(0, 1 << 20);
    assert!(
        !straight_away.text.contains("late-output"),
        "the write should not have waited for the command: {:?}",
        straight_away.text
    );

    let seen = wait_for(&terminal, "late-output").await;
    close(&terminal.id);

    assert!(
        seen.contains("late-output"),
        "output produced after the call returned was never collected: {seen:?}"
    );
}

/// Reading from the offset the last read returned gives only what is new â€”
/// the mechanism that lets an agent check on a process without re-reading
/// everything it has ever printed.
#[tokio::test]
#[serial_test::serial]
async fn a_resumed_read_sees_only_new_output() {
    let terminal = open("session-resume");
    terminal.write("echo one\n");
    wait_for(&terminal, "one").await;

    let mark = terminal.produced();
    terminal.write("echo two\n");
    let seen = wait_for(&terminal, "two").await;
    assert!(seen.contains("two"), "{seen:?}");

    let fresh = terminal.read(mark, 1 << 20);
    close(&terminal.id);

    assert!(fresh.text.contains("two"), "{:?}", fresh.text);
    assert!(
        !fresh.text.contains("one\n"),
        "output from before the mark came back: {:?}",
        fresh.text
    );
}

#[tokio::test]
#[serial_test::serial]
async fn session_end_closes_that_sessions_terminals_and_leaves_others_alone() {
    let mine = open("session-ending");
    let theirs = open("session-continuing");

    let closed = close_session("session-ending");

    assert_eq!(closed, 1);
    assert!(get(&mine.id).is_none(), "my terminal should be gone");
    assert!(
        get(&theirs.id).is_some(),
        "another session's terminal must survive"
    );

    close(&theirs.id);
}

#[tokio::test]
#[serial_test::serial]
async fn an_idle_terminal_is_reaped_and_a_busy_one_is_not() {
    let stale = open("session-idle");
    let fresh = open("session-idle");
    fresh.write("echo still here\n");

    // A timeout of zero would reap both; the point is that recency decides, so
    // the clock is moved forward past only the untouched one.
    tokio::time::sleep(Duration::from_millis(50)).await;
    fresh.read(0, 64);
    let reaped = reap_idle_at(Instant::now(), Duration::from_millis(40));

    assert!(
        reaped >= 1,
        "the untouched terminal should have been reaped"
    );
    assert!(get(&stale.id).is_none(), "stale terminal survived");
    assert!(
        get(&fresh.id).is_some(),
        "the just-read terminal was reaped"
    );

    close(&fresh.id);
}

/// The cap is process-wide, so this test has to have the registry to itself.
#[tokio::test]
#[serial_test::serial]
async fn the_cap_refuses_one_more_and_says_how_to_make_room() {
    let existing: Vec<String> = TERMINALS.iter().map(|entry| entry.key().clone()).collect();
    for id in &existing {
        close(id);
    }

    let opened: Vec<Arc<Terminal>> = (0..MAX_TERMINALS).map(|_| open("session-cap")).collect();
    assert_eq!(opened.len(), MAX_TERMINALS);

    let program = crate::terminal_shell::build(shell_name(), &std::env::temp_dir()).expect("shell");
    let Err(refused) = create(
        "session-cap",
        "test-one-too-many".to_string(),
        shell_name().to_string(),
        false,
        program,
    ) else {
        panic!("the cap must hold");
    };

    assert!(refused.contains("TerminalClose"), "{refused}");
    assert_eq!(close_session("session-cap"), MAX_TERMINALS);
}
