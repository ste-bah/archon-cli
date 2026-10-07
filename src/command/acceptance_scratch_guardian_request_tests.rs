//! The guardian's request reader, driven in-process with a short deadline.
//!
//! Issue 302: the only proof of the read deadline used to be a subprocess test
//! whose wall-clock window also had to cover process start of the whole test
//! binary, so a loaded runner failed it. Here the deadline is a parameter and
//! the clock starts at the call, so start-up cost cannot reach the assertion.
use std::io::Write;
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Runs the reader on its own thread so a reader that never returns fails the
/// test at `ceiling` instead of hanging it.
fn read_with_limit(
    reader: std::io::PipeReader,
    limit: Duration,
    ceiling: Duration,
) -> (archon_workflow::WorkflowResult<String>, Duration) {
    let (sender, outcome) = mpsc::channel();
    std::thread::spawn(move || {
        let started = Instant::now();
        let result = super::read_request(&reader, limit);
        let _ = sender.send((result, started.elapsed()));
    });
    outcome
        .recv_timeout(ceiling)
        .expect("the request reader never returned")
}

/// A parent that writes part of a line and then stalls, pipe still open, is
/// paused by the no-progress window: not by end-of-file, not before it.
#[test]
fn a_stalled_partial_request_is_refused_at_the_read_deadline() {
    let (reader, mut writer) = std::io::pipe().unwrap();
    writer.write_all(b"{").unwrap();
    let limit = Duration::from_millis(200);
    let (result, elapsed) = read_with_limit(reader, limit, limit * 150);
    drop(writer);
    let error = result.expect_err("a partial request was accepted");
    assert!(
        matches!(&error, archon_workflow::WorkflowError::ControlPaused(why) if why.contains("no guardian request byte")),
        "a stall pauses, never fails: {error}"
    );
    assert!(
        elapsed >= limit,
        "refused after {elapsed:?}, inside {limit:?}"
    );
}

/// The deadline bounds a stall only: a complete line is returned at once.
#[test]
fn a_complete_request_line_is_read_without_its_newline() {
    let (reader, mut writer) = std::io::pipe().unwrap();
    writer.write_all(b"{\"k\":1}\n").unwrap();
    let limit = Duration::from_secs(60);
    let (result, _) = read_with_limit(reader, limit, limit * 2);
    drop(writer);
    assert_eq!(result.unwrap(), "{\"k\":1}");
}

/// #356: the window renews on every byte: a line that keeps arriving, each
/// gap below the window, is read whole however long it takes in total.
#[test]
fn issue356_a_slow_but_progressing_request_line_is_read_whole() {
    let (reader, mut writer) = std::io::pipe().unwrap();
    let limit = Duration::from_millis(400);
    let feeder = std::thread::spawn(move || {
        for byte in b"{\"slow\":1}\n" {
            std::thread::sleep(Duration::from_millis(100));
            writer.write_all(&[*byte]).unwrap();
        }
        writer
    });
    let started = Instant::now();
    let (result, _) = read_with_limit(reader, limit, Duration::from_secs(60));
    assert!(
        started.elapsed() > limit * 2,
        "the total outlasted the window"
    );
    drop(feeder.join().unwrap());
    assert_eq!(result.unwrap(), "{\"slow\":1}");
}

/// #356: a stall after progress is measured from the last byte, not the start.
#[test]
fn issue356_a_request_stall_is_measured_from_its_last_byte() {
    let (reader, mut writer) = std::io::pipe().unwrap();
    let limit = Duration::from_millis(400);
    let feeder = std::thread::spawn(move || {
        for _ in 0..4 {
            std::thread::sleep(Duration::from_millis(150));
            writer.write_all(b"x").unwrap();
        }
        writer
    });
    let (result, elapsed) = read_with_limit(reader, limit, Duration::from_secs(60));
    drop(feeder.join().unwrap());
    assert!(
        matches!(
            result,
            Err(archon_workflow::WorkflowError::ControlPaused(_))
        ),
        "{result:?}"
    );
    assert!(elapsed >= Duration::from_millis(600) + limit, "{elapsed:?}");
}
