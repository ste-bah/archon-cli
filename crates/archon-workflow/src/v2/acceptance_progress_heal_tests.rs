//! Round 8 of the Issue 262 review (P1): a damaged round record never
//! fails the load. It is moved aside with its evidence, never deleted,
//! and the ledger is rebuilt from what remains.

use super::super::{attempt_file_name, next_attempt, round_dir, write_round_record};
use super::tests::set;
use super::*;

/// X, X, Y recorded the way the host records them: each record, then the
/// ledger that observed it.
fn recorded_with_ledger() -> (tempfile::TempDir, Vec<AcceptanceRoundRecordV1>) {
    let dir = tempfile::tempdir().unwrap();
    let records = vec![set(1, 1, "AC-X"), set(2, 1, "AC-X"), set(1, 2, "AC-Y")];
    let mut ledger = ProgressLedger::default();
    for record in &records {
        write_round_record(dir.path(), record).unwrap();
        ledger.observe(record);
        ledger.save(dir.path()).unwrap();
    }
    (dir, records)
}

fn file_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = (std::fs::read_dir(dir).unwrap())
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// The damaged record's state comes from the ledger's copy, so the rebuild
/// is exact: neither a stall read as progress nor progress read as a stall.
/// The damaged bytes are kept beside evidence naming the record, and a
/// later load (the record gone from its round) rebuilds the same ledger.
#[test]
fn a_corrupt_record_is_quarantined_and_rebuilt_from_its_ledger_copy() {
    let (dir, records) = recorded_with_ledger();
    let damaged = round_dir(dir.path(), 2).join(attempt_file_name(1));
    std::fs::write(&damaged, "{\"round\":").unwrap();
    let expected = ProgressLedger::from_history(&records);

    let rebuilt =
        ProgressLedger::load(dir.path(), 2).expect("a corrupt record never fails the load");

    assert_eq!(rebuilt, expected);
    assert!(!damaged.exists(), "the damaged record is moved aside");
    let quarantine = round_dir(dir.path(), 2).join("quarantine");
    let names = file_names(&quarantine);
    let kept = (names.iter())
        .map(|name| quarantine.join(name))
        .find(|path| std::fs::read(path).unwrap() == b"{\"round\":")
        .unwrap_or_else(|| panic!("the damaged bytes are kept: {names:?}"));
    let evidence = (names.iter())
        .map(|name| quarantine.join(name))
        .filter(|path| *path != kept)
        .map(|path| serde_json::from_slice::<serde_json::Value>(&std::fs::read(path).unwrap()))
        .find_map(Result::ok)
        .unwrap_or_else(|| panic!("evidence beside it: {names:?}"));
    assert_eq!(evidence["round"], 2, "{evidence:#}");
    assert_eq!(evidence["attempt"], 1, "{evidence:#}");
    assert_eq!(
        evidence["state"],
        serde_json::json!(["AC-X"]),
        "{evidence:#}"
    );
    assert!(
        evidence["reason"]
            .as_str()
            .is_some_and(|why| !why.is_empty())
    );
    assert_eq!(ProgressLedger::load(dir.path(), 2).unwrap(), expected);
    assert_eq!(
        next_attempt(dir.path(), 2),
        2,
        "a quarantined attempt number is never reused"
    );
}

/// With no copy of the damaged record's state, the record is still moved
/// aside (never left to fail every later load) and the loss is reported:
/// the caller pauses, it never guesses.
#[test]
fn a_corrupt_record_with_no_copy_is_quarantined_and_reported_unknown() {
    let dir = x_x_y_without_ledger();
    let damaged = round_dir(dir.path(), 2).join(attempt_file_name(1));
    std::fs::write(&damaged, "{\"round\":").unwrap();

    let healed = ProgressLedger::load_healing(dir.path()).expect("never an error");

    assert!(!damaged.exists());
    assert_eq!(healed.quarantined.len(), 1);
    assert_eq!(healed.unknown().len(), 1, "{:?}", healed.quarantined);
    assert_eq!(
        healed.ledger,
        ProgressLedger::from_history(&[set(1, 1, "AC-X"), set(1, 2, "AC-Y")])
    );
    let again = ProgressLedger::load_healing(dir.path()).unwrap();
    assert!(again.quarantined.is_empty(), "reported once, at discovery");
}

fn x_x_y_without_ledger() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for record in [set(1, 1, "AC-X"), set(2, 1, "AC-X"), set(1, 2, "AC-Y")] {
        write_round_record(dir.path(), &record).unwrap();
    }
    dir
}

/// A record the file system will not hand over is no damage to quarantine:
/// it is an I/O error the caller pauses on, and the record stays in place.
#[cfg(unix)]
#[test]
fn an_unreadable_record_is_an_io_error_and_stays_in_place() {
    use std::os::unix::fs::PermissionsExt;
    let (dir, _) = recorded_with_ledger();
    let record = round_dir(dir.path(), 2).join(attempt_file_name(1));
    std::fs::set_permissions(&record, std::fs::Permissions::from_mode(0o000)).unwrap();
    let loaded = ProgressLedger::load(dir.path(), 2);
    std::fs::set_permissions(&record, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(
        matches!(&loaded, Err(crate::WorkflowError::Io { .. })),
        "{loaded:?}"
    );
    assert!(record.exists());
    assert!(!round_dir(dir.path(), 2).join("quarantine").exists());
}

/// Round 9 (P2): a record quarantined with no copy of its state is a loss
/// the caller must pause on. A process that dies after the move and before
/// the pause must not leave the next load silent: the loss is reported
/// again on every load until a pause acknowledges it, and only then.
#[test]
fn an_unacknowledged_unknown_loss_is_reported_again_after_a_crash() {
    let dir = x_x_y_without_ledger();
    std::fs::write(
        round_dir(dir.path(), 2).join(attempt_file_name(1)),
        "{\"round\":",
    )
    .unwrap();
    let first = ProgressLedger::load_healing(dir.path()).unwrap();
    assert_eq!(first.unknown().len(), 1);

    // The process died here, before it paused.
    let again = ProgressLedger::load_healing(dir.path()).unwrap();

    assert!(again.quarantined.is_empty(), "the event is reported once");
    let [lost] = &again.unknown()[..] else {
        panic!("the unacknowledged loss is reported again: {again:?}");
    };
    assert_eq!((lost.round, lost.attempt), (2, 1));
    acknowledge_quarantined(dir.path(), &[(*lost).clone()]).unwrap();
    let acknowledged = ProgressLedger::load_healing(dir.path()).unwrap();
    assert!(acknowledged.unknown().is_empty(), "{acknowledged:?}");
    assert_eq!(acknowledged.ledger, again.ledger);
}

/// Round 9 (P2): a legacy ledger (no `observed` copy) is the history of a
/// run from before the records. Once every record is quarantined it must
/// not come back: its revisit count is the very state that was lost.
#[test]
fn a_legacy_ledger_never_returns_once_every_record_is_quarantined() {
    let dir = tempfile::tempdir().unwrap();
    for record in [set(1, 1, "AC-X"), set(2, 1, "AC-X")] {
        write_round_record(dir.path(), &record).unwrap();
    }
    let legacy = dir
        .path()
        .join(ACCEPTANCE_RECORDS_DIR)
        .join(PROGRESS_LEDGER_FILE);
    std::fs::write(&legacy, r#"{"seen":[["AC-X"]],"revisits":1}"#).unwrap();
    for round in [1, 2] {
        std::fs::write(round_dir(dir.path(), round).join(attempt_file_name(1)), "{").unwrap();
    }
    let first = ProgressLedger::load_healing(dir.path()).unwrap();
    assert_eq!(first.ledger, ProgressLedger::default(), "{first:?}");

    let again = ProgressLedger::load_healing(dir.path()).unwrap();

    assert_eq!(
        again.ledger,
        ProgressLedger::default(),
        "the lost revisit count never returns from the legacy ledger"
    );
}

/// Round 9 (P1): two writers of one attempt never both succeed. Both are
/// made to wait on the order lock after deciding which attempt to write;
/// the existence check happens under the lock, so the second one finds the
/// first one's record and is refused, and the record on disk is the one
/// whose writer succeeded.
#[test]
fn two_writers_of_one_attempt_never_both_succeed() {
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
    let writers: Vec<_> = ["AC-A", "AC-B"]
        .into_iter()
        .map(|id| {
            let run_dir = dir.path().to_path_buf();
            std::thread::spawn(move || (id, write_round_record(&run_dir, &set(1, 1, id))))
        })
        .collect();
    std::thread::sleep(std::time::Duration::from_millis(300));
    drop(held);
    let outcomes: Vec<_> = writers.into_iter().map(|w| w.join().unwrap()).collect();

    let won: Vec<&str> = (outcomes.iter())
        .filter(|(_, outcome)| outcome.is_ok())
        .map(|(id, _)| *id)
        .collect();
    let [winner] = won[..] else {
        panic!("exactly one writer of an attempt succeeds: {outcomes:?}");
    };
    let path = round_dir(dir.path(), 1).join(attempt_file_name(1));
    let on_disk: AcceptanceRoundRecordV1 =
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(state_key(&on_disk), vec![winner.to_string()]);
}
