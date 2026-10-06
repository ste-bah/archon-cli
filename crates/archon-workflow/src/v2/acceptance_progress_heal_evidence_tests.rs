//! Issue 317: quarantine evidence that will not parse is never skipped.
//! Its record is a loss of unknown state, reported until a pause
//! acknowledges it; the legacy ledger never stands in for it.

use super::super::{attempt_file_name, round_dir, write_round_record};
use super::tests::set;
use super::*;
use std::path::PathBuf;

const DAMAGED_EVIDENCE: &[u8] = b"{\"event\":";

fn legacy_ledger(run_dir: &Path) {
    let path = run_dir
        .join(ACCEPTANCE_RECORDS_DIR)
        .join(PROGRESS_LEDGER_FILE);
    std::fs::write(path, r#"{"seen":[["AC-X"]],"revisits":1}"#).unwrap();
}

/// Records X (1,1), X (2,1), Y (1,2); (2,1) is then damaged and
/// quarantined by a load, and its evidence damaged after it. Returns the
/// quarantined record as the first load reported it.
fn quarantined_then_evidence_damaged(run_dir: &Path, with_copy: bool) -> QuarantinedRecordV1 {
    let mut ledger = ProgressLedger::default();
    for record in [set(1, 1, "AC-X"), set(2, 1, "AC-X"), set(1, 2, "AC-Y")] {
        write_round_record(run_dir, &record).unwrap();
        ledger.observe(&record);
        if with_copy {
            ledger.save(run_dir).unwrap();
        }
    }
    std::fs::write(round_dir(run_dir, 2).join(attempt_file_name(1)), "{").unwrap();
    let first = ProgressLedger::load_healing(run_dir).unwrap();
    let [quarantined] = &first.quarantined[..] else {
        panic!("one record quarantined: {first:?}");
    };
    std::fs::write(evidence_path(run_dir, quarantined), DAMAGED_EVIDENCE).unwrap();
    quarantined.clone()
}

fn evidence_path(run_dir: &Path, quarantined: &QuarantinedRecordV1) -> PathBuf {
    let moved = run_dir.join(&quarantined.quarantined);
    let name = moved.file_name().unwrap().to_str().unwrap();
    moved.with_file_name(name.replace(".damaged", ".evidence.json"))
}

fn assert_lost(healed: &HealedLedger, quarantined: &QuarantinedRecordV1) {
    let [lost] = &healed.unknown()[..] else {
        panic!("the record of damaged evidence is a reported loss: {healed:?}");
    };
    assert_eq!((lost.round, lost.attempt), (2, 1));
    assert_eq!(lost.original, quarantined.original);
    assert_eq!(lost.quarantined, quarantined.quarantined);
    assert_eq!(lost.state, None);
    assert!(lost.reason.contains("evidence"), "{}", lost.reason);
}

/// No ledger at all: the record of the damaged evidence is reported lost,
/// and the rebuild holds only what is known.
#[test]
fn damaged_evidence_is_a_lost_record_of_unknown_state() {
    let dir = tempfile::tempdir().unwrap();
    let quarantined = quarantined_then_evidence_damaged(dir.path(), false);

    let healed = ProgressLedger::load_healing(dir.path()).unwrap();

    assert_lost(&healed, &quarantined);
    assert_eq!(
        healed.ledger,
        ProgressLedger::from_history(&[set(1, 1, "AC-X"), set(1, 2, "AC-Y")])
    );
}

/// The review's probe: a legacy ledger (no copy of any state) and every
/// record quarantined, one of them with damaged evidence. The legacy
/// revisit count never returns, and the loss is reported.
#[test]
fn damaged_evidence_with_a_legacy_ledger_never_brings_it_back() {
    let dir = tempfile::tempdir().unwrap();
    write_round_record(dir.path(), &set(2, 1, "AC-X")).unwrap();
    legacy_ledger(dir.path());
    std::fs::write(round_dir(dir.path(), 2).join(attempt_file_name(1)), "{").unwrap();
    let first = ProgressLedger::load_healing(dir.path()).unwrap();
    let quarantined = first.quarantined[0].clone();
    std::fs::write(evidence_path(dir.path(), &quarantined), DAMAGED_EVIDENCE).unwrap();

    let healed = ProgressLedger::load_healing(dir.path()).unwrap();

    assert_lost(&healed, &quarantined);
    assert_eq!(
        healed.ledger,
        ProgressLedger::default(),
        "never the legacy ledger's revisit count"
    );
}

/// The saved ledger's copy holds the state: the damaged evidence costs
/// nothing, the rebuild is exact and nothing is reported.
#[test]
fn damaged_evidence_with_a_ledger_copy_rebuilds_exactly() {
    let dir = tempfile::tempdir().unwrap();
    quarantined_then_evidence_damaged(dir.path(), true);

    let healed = ProgressLedger::load_healing(dir.path()).unwrap();

    assert!(healed.unknown().is_empty(), "{healed:?}");
    assert_eq!(
        healed.ledger,
        ProgressLedger::from_history(&[set(1, 1, "AC-X"), set(2, 1, "AC-X"), set(1, 2, "AC-Y")])
    );
}

/// The pause acknowledges the loss: the evidence is replaced, its damaged
/// bytes kept beside it, and the next load reports nothing and still never
/// brings back the legacy ledger.
#[test]
fn acknowledging_damaged_evidence_keeps_its_bytes_and_ends_the_report() {
    let dir = tempfile::tempdir().unwrap();
    let quarantined = quarantined_then_evidence_damaged(dir.path(), false);
    legacy_ledger(dir.path());
    let healed = ProgressLedger::load_healing(dir.path()).unwrap();
    let lost: Vec<QuarantinedRecordV1> = healed.unknown().into_iter().cloned().collect();

    acknowledge_quarantined(dir.path(), &lost).unwrap();

    let evidence = evidence_path(dir.path(), &quarantined);
    let name = evidence.file_name().unwrap().to_str().unwrap();
    let stem = name.strip_suffix(".evidence.json").unwrap();
    let kept = evidence.with_file_name(format!("{stem}.evidence.unreadable"));
    assert_eq!(std::fs::read(kept).unwrap(), DAMAGED_EVIDENCE);
    let again = ProgressLedger::load_healing(dir.path()).unwrap();
    assert!(again.unknown().is_empty(), "{again:?}");
    assert_eq!(again.ledger, healed.ledger);
    assert_ne!(again.ledger.revisits, 1, "never the legacy count");
}

/// Damaged evidence whose record never moved (a crash between the
/// evidence and the move) is no loss: the record is still in its round
/// and counts there.
#[test]
fn damaged_evidence_of_a_record_that_never_moved_is_no_loss() {
    let dir = tempfile::tempdir().unwrap();
    write_round_record(dir.path(), &set(1, 1, "AC-X")).unwrap();
    let quarantine = round_dir(dir.path(), 1).join(QUARANTINE_DIR);
    std::fs::create_dir_all(&quarantine).unwrap();
    std::fs::write(
        quarantine.join("attempt-01.json.0000.evidence.json"),
        DAMAGED_EVIDENCE,
    )
    .unwrap();

    let healed = ProgressLedger::load_healing(dir.path()).unwrap();

    assert!(healed.unknown().is_empty(), "{healed:?}");
    assert_eq!(
        healed.ledger,
        ProgressLedger::from_history(&[set(1, 1, "AC-X")])
    );
}

/// A quarantine the file system will not list is an I/O error the caller
/// pauses on, never an empty quarantine.
#[cfg(unix)]
#[test]
fn an_unreadable_quarantine_directory_is_an_io_error() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    quarantined_then_evidence_damaged(dir.path(), false);
    legacy_ledger(dir.path());
    let quarantine = round_dir(dir.path(), 2).join(QUARANTINE_DIR);
    std::fs::set_permissions(&quarantine, std::fs::Permissions::from_mode(0o000)).unwrap();

    let loaded = ProgressLedger::load_healing(dir.path());

    std::fs::set_permissions(&quarantine, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(
        matches!(&loaded, Err(crate::WorkflowError::Io { .. })),
        "{loaded:?}"
    );
}

/// Review A5: when the file system will not say whether the record's bytes
/// moved (here a link that loops), the load is an I/O error the caller
/// pauses on, never a record taken as never moved.
#[cfg(unix)]
#[test]
fn an_unknowable_move_is_an_io_error_never_no_loss() {
    let dir = tempfile::tempdir().unwrap();
    let quarantined = quarantined_then_evidence_damaged(dir.path(), false);
    let moved = dir.path().join(&quarantined.quarantined);
    std::fs::rename(&moved, dir.path().join("kept.damaged")).unwrap();
    std::os::unix::fs::symlink(&moved, &moved).unwrap();

    let loaded = ProgressLedger::load_healing(dir.path());

    assert!(
        matches!(&loaded, Err(crate::WorkflowError::Io { .. })),
        "{loaded:?}"
    );
}

/// The same for evidence that parses: whether its record's bytes moved is
/// read with the error passed up, never taken as "never moved".
#[cfg(unix)]
#[test]
fn an_unknowable_move_of_readable_evidence_is_an_io_error() {
    let dir = tempfile::tempdir().unwrap();
    write_round_record(dir.path(), &set(1, 1, "AC-X")).unwrap();
    std::fs::write(round_dir(dir.path(), 1).join(attempt_file_name(1)), "{").unwrap();
    let first = ProgressLedger::load_healing(dir.path()).unwrap();
    let moved = dir.path().join(&first.quarantined[0].quarantined);
    std::fs::rename(&moved, dir.path().join("kept.damaged")).unwrap();
    std::os::unix::fs::symlink(&moved, &moved).unwrap();

    let loaded = ProgressLedger::load_healing(dir.path());

    assert!(
        matches!(&loaded, Err(crate::WorkflowError::Io { .. })),
        "{loaded:?}"
    );
}

#[test]
fn gc_missing_damaged_bytes_do_not_hide_unreadable_evidence() {
    let mut refused = Vec::new();
    for with_copy in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let lost = quarantined_then_evidence_damaged(dir.path(), with_copy);
        std::fs::remove_file(dir.path().join(&lost.quarantined)).unwrap();
        refused.push(ProgressLedger::load_healing(dir.path()).is_err());
    }
    let dir = tempfile::tempdir().unwrap();
    let quarantine = round_dir(dir.path(), 1).join("quarantine");
    std::fs::create_dir_all(&quarantine).unwrap();
    std::fs::write(quarantine.join("attempt-01.json.lost.evidence.json"), b"{").unwrap();
    refused.push(ProgressLedger::load_healing(dir.path()).is_err());
    assert!(
        refused.iter().all(|refused| *refused),
        "missing bytes hid evidence damage: {refused:?}"
    );
}

fn r2_parseable_missing_bytes(copy: bool, conflicting_original: bool) {
    let dir = tempfile::tempdir().unwrap();
    let mut ledger = ProgressLedger::default();
    for record in [set(1, 1, "AC-X"), set(2, 1, "AC-X")] {
        write_round_record(dir.path(), &record).unwrap();
        ledger.observe(&record);
    }
    if copy {
        ledger.save(dir.path()).unwrap();
    }
    let original = round_dir(dir.path(), 2).join(attempt_file_name(1));
    std::fs::write(&original, b"{").unwrap();
    let first = ProgressLedger::load_healing(dir.path()).unwrap();
    let lost = &first.quarantined[0];
    let moved = dir.path().join(&lost.quarantined);
    let evidence = moved.with_file_name(
        moved
            .file_name()
            .unwrap()
            .to_string_lossy()
            .replace(".damaged", ".evidence.json"),
    );
    // The named case has valid evidence, missing bytes, and another intact round.
    let parsed: QuarantinedRecordV1 =
        serde_json::from_slice(&std::fs::read(&evidence).unwrap()).unwrap();
    assert_eq!(parsed.round, 2);
    std::fs::remove_file(&moved).unwrap();
    if conflicting_original {
        std::fs::write(&original, serde_json::to_vec(&set(99, 1, "AC-X")).unwrap()).unwrap();
    }
    let healed = ProgressLedger::load_healing(dir.path()).unwrap();
    assert_eq!(
        healed.unknown().len(),
        1,
        "parseable evidence byte loss silently skipped"
    );
    assert!(healed.unknown()[0].reason.contains("missing"));
    if copy {
        assert!(
            healed
                .ledger
                .observed
                .iter()
                .any(|o| o.round == 2 && o.attempt == 1),
            "known observation must remain"
        );
        assert!(healed.ledger.revisits >= 1, "known revisit must remain");
        if !conflicting_original {
            assert_eq!(healed.ledger, ledger);
        }
    }
    assert_eq!(
        ProgressLedger::load_healing(dir.path())
            .unwrap()
            .unknown()
            .len(),
        1
    );
    acknowledge_quarantined(dir.path(), &healed.unacknowledged).unwrap();
    if copy {
        std::fs::remove_file(
            dir.path()
                .join(ACCEPTANCE_RECORDS_DIR)
                .join(PROGRESS_LEDGER_FILE),
        )
        .unwrap();
    }
    let acknowledged = ProgressLedger::load_healing(dir.path()).unwrap();
    assert!(acknowledged.unknown().is_empty());
    assert_eq!(
        acknowledged.ledger, healed.ledger,
        "acknowledgment must keep the known observation after saved-copy loss"
    );
}
#[test]
fn r2_parseable_byte_loss_with_saved_observation() {
    r2_parseable_missing_bytes(true, false);
}
#[test]
fn r2_parseable_byte_loss_without_saved_observation() {
    r2_parseable_missing_bytes(false, false);
}
#[test]
fn r2_parseable_byte_loss_with_nonmatching_original() {
    r2_parseable_missing_bytes(true, true);
}
