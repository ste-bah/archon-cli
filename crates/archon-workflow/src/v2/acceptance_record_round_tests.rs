//! Issue 316: [`record_round`] never refuses its writer for a taken
//! attempt number; a writer its decision refuses lands nothing.

use super::*;

fn failing(round: u32, attempt: u32, check: &str) -> AcceptanceRoundRecordV1 {
    serde_json::from_value(serde_json::json!({
        "schema_version": 1,
        "run_id": "run",
        "call_id": format!("{ACCEPTANCE_STAGE_CALL_PREFIX}{round}"),
        "round": round,
        "attempt": attempt,
        "max_rounds": 3,
        "contract_present": true,
        "checks": [{
            "check_id": check,
            "criterion": "criterion",
            "kind": "command",
            "status": "failed",
            "owning_tasks": ["TASK-1"],
        }],
        "final_round": false,
    }))
    .unwrap()
}

fn on_disk(run_dir: &Path, attempt: u32) -> AcceptanceRoundRecordV1 {
    let path = round_dir(run_dir, 1).join(attempt_file_name(attempt));
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

fn accept(
    record: &mut AcceptanceRoundRecordV1,
    landing: &mut RoundLanding<'_>,
) -> WorkflowResult<u32> {
    landing.land(record)?;
    Ok(record.attempt)
}

/// Both writers chose attempt 1 and wait on the order lock: both land, at
/// 1 and 2, each record whole and saying the number it landed as, and the
/// decision of each saw that number.
#[test]
fn two_writers_of_one_attempt_both_land_at_distinct_numbers() {
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
            std::thread::spawn(move || {
                let mut record = failing(1, 1, id);
                let landed = record_round(&run_dir, &mut record, accept).unwrap();
                (id, record.attempt, landed)
            })
        })
        .collect();
    std::thread::sleep(std::time::Duration::from_millis(300));
    drop(held);
    let mut outcomes: Vec<_> = writers.into_iter().map(|w| w.join().unwrap()).collect();
    outcomes.sort_by_key(|(_, attempt, _)| *attempt);

    let attempts: Vec<u32> = outcomes.iter().map(|(_, attempt, _)| *attempt).collect();
    assert_eq!(attempts, [1, 2], "{outcomes:?}");
    for (id, attempt, (path, decided)) in &outcomes {
        assert_eq!(*decided, *attempt, "decided with the number it lands as");
        assert!(path.ends_with(attempt_file_name(*attempt)));
        let record = on_disk(dir.path(), *attempt);
        assert_eq!(
            (record.attempt, record.checks[0].check_id.as_str()),
            (*attempt, *id)
        );
    }
}

/// A number taken by a quarantined record is taken too: the record takes
/// the next one.
#[test]
fn a_quarantined_number_is_skipped() {
    let dir = tempfile::tempdir().unwrap();
    write_round_record(dir.path(), &failing(1, 1, "AC-A")).unwrap();
    std::fs::write(round_dir(dir.path(), 1).join(attempt_file_name(1)), "{").unwrap();
    progress::ProgressLedger::load_healing(dir.path()).unwrap();
    let mut record = failing(1, 1, "AC-B");

    let (path, _) = record_round(dir.path(), &mut record, accept).unwrap();

    assert_eq!(record.attempt, 2);
    assert!(path.ends_with(attempt_file_name(2)));
}

/// A writer its decision refuses (an obsolete generation) lands nothing:
/// no record and no recording-order entry, and its number stays free.
#[test]
fn a_refused_writer_lands_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mut record = failing(1, 1, "AC-A");

    let refused = record_round(dir.path(), &mut record, |_, _| {
        WorkflowResult::<()>::Err(WorkflowError::ControlCancelled("obsolete".into()))
    });

    assert!(matches!(refused, Err(WorkflowError::ControlCancelled(_))));
    assert!(!round_dir(dir.path(), 1).join(attempt_file_name(1)).exists());
    let log = dir
        .path()
        .join(ACCEPTANCE_RECORDS_DIR)
        .join("recording-order.log");
    assert!(std::fs::read_to_string(log).unwrap_or_default().is_empty());
    assert_eq!(next_attempt(dir.path(), 1), 1);
}

/// An act that never lands, or lands twice, is refused: a decided record
/// never goes unwritten in silence, and one landing is one record.
#[test]
fn a_landing_is_made_exactly_once() {
    let dir = tempfile::tempdir().unwrap();
    let mut record = failing(1, 1, "AC-A");

    let unlanded = record_round(dir.path(), &mut record, |_, _| WorkflowResult::Ok(()));
    let twice = record_round(dir.path(), &mut record, |record, landing| {
        landing.land(record)?;
        landing.land(record)
    });

    assert!(
        matches!(unlanded, Err(WorkflowError::StateCorrupt(_))),
        "{unlanded:?}"
    );
    assert!(
        matches!(twice, Err(WorkflowError::StateCorrupt(_))),
        "{twice:?}"
    );
    assert!(round_dir(dir.path(), 1).join(attempt_file_name(1)).exists());
    assert!(!round_dir(dir.path(), 1).join(attempt_file_name(2)).exists());
}
