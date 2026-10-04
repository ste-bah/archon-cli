//! Round 8 of the Issue 262 review: the recording-order log under
//! concurrent writers and torn lines.

use std::collections::BTreeSet;

use super::super::tests::{assert_replayed_x_x_y, order_log, x_x_y};
use super::note_recorded;
use crate::v2::acceptance_stage::ACCEPTANCE_RECORDS_DIR;

/// Every line of `text` as (seq, round, attempt), panicking on a line no
/// writer wrote whole.
fn whole_lines(text: &str) -> Vec<(u64, u32, u32)> {
    (text.lines())
        .map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            let [seq, round, attempt, "end"] = fields[..] else {
                panic!("line {line:?} is torn or interleaved in {text:?}");
            };
            let seq = seq.strip_prefix("seq=").and_then(|seq| seq.parse().ok());
            let (Some(seq), Ok(round), Ok(attempt)) = (seq, round.parse(), attempt.parse()) else {
                panic!("line {line:?} is torn or interleaved in {text:?}");
            };
            (seq, round, attempt)
        })
        .collect()
}

/// Writers that append at once each leave one whole line, and the sequence
/// numbers rise strictly in the order the lines landed.
#[test]
fn concurrent_appenders_leave_whole_lines_in_strictly_increasing_order() {
    let dir = tempfile::tempdir().unwrap();
    let writers = 64_u32;
    let barrier = std::sync::Barrier::new(writers as usize);
    std::thread::scope(|scope| {
        for attempt in 1..=writers {
            let (run_dir, barrier) = (dir.path(), &barrier);
            scope.spawn(move || {
                barrier.wait();
                note_recorded(run_dir, 1, attempt);
            });
        }
    });
    let text = std::fs::read_to_string(order_log(dir.path())).unwrap();
    assert!(text.ends_with('\n'), "{text:?}");
    let lines = whole_lines(&text);
    let attempts: BTreeSet<u32> = lines.iter().map(|(_, _, attempt)| *attempt).collect();
    assert_eq!(
        attempts.len(),
        writers as usize,
        "every append kept: {text:?}"
    );
    assert!(
        lines.windows(2).all(|pair| pair[0].0 < pair[1].0),
        "sequence numbers rise strictly: {text:?}"
    );
}

/// A writer that has to wait never appends a sequence number at or below
/// one already in the log: it reads the log only once it holds the lock,
/// so the entries other writers appended meanwhile come first.
#[test]
fn a_delayed_writer_waits_and_never_reorders_the_log() {
    let dir = tempfile::tempdir().unwrap();
    let records = dir.path().join(ACCEPTANCE_RECORDS_DIR);
    std::fs::create_dir_all(&records).unwrap();
    let lock_file = (std::fs::OpenOptions::new())
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(records.join("recording-order.lock"))
        .unwrap();
    let mut lock = fd_lock::RwLock::new(lock_file);
    let held = lock.write().unwrap();
    let writer = std::thread::spawn({
        let run_dir = dir.path().to_path_buf();
        move || note_recorded(&run_dir, 1, 2)
    });
    std::thread::sleep(std::time::Duration::from_millis(300));
    let log = order_log(dir.path());
    let early = std::fs::read_to_string(&log).unwrap_or_default();
    // Two writers that hold the lock meanwhile append X, X.
    std::fs::write(&log, "seq=1 1 1 end\nseq=2 2 1 end\n").unwrap();
    drop(held);
    writer.join().unwrap();
    assert!(
        early.is_empty(),
        "a writer appends only under the lock: {early:?}"
    );
    let text = std::fs::read_to_string(&log).unwrap();
    assert_eq!(
        whole_lines(&text),
        vec![(1, 1, 1), (2, 2, 1), (3, 1, 2)],
        "{text:?}"
    );
}

/// A log a racing writer of round 7 left with a sequence number repeated
/// (A reserved 1, B and C appended 1 and 2, A appended 1) replays in the
/// order the lines landed: X, X, Y, never X, Y, X.
#[test]
fn a_repeated_sequence_number_replays_in_landing_order() {
    let log = "seq=1 1 1 end\nseq=2 2 1 end\nseq=1 1 2 end\n";
    assert_replayed_x_x_y(x_x_y([1000, 2000, 3000], Some(log)).path());
}

/// The review's reproduction: a legacy writer crashed while writing round
/// 2 attempt 12, leaving the unterminated fragment `2 1`. Neither replay
/// nor the next append may take it as the entry of round 2 attempt 1, in
/// any format.
#[test]
fn an_unterminated_last_line_is_never_an_entry() {
    for log in [
        "1 1\n2 1\n1 2\n2 1",
        "1 1 10\n2 1 20\n1 2 30\n2 1 4",
        "seq=1 1 1 end\nseq=2 2 1 end\nseq=3 1 2 end\nseq=4 2 1 end",
    ] {
        let dir = x_x_y([1000, 2000, 3000], Some(log));
        assert_replayed_x_x_y(dir.path());
        note_recorded(dir.path(), 1, 77);
        assert_replayed_x_x_y(dir.path());
        let text = std::fs::read_to_string(order_log(dir.path())).unwrap();
        assert!(text.ends_with('\n'), "{text:?}");
        assert!(
            text.lines()
                .last()
                .is_some_and(|line| line.ends_with(" 1 77 end")),
            "{text:?}"
        );
    }
}
