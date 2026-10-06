use super::super::progress::tests::set;
use super::*;

#[test]
fn gc_round_reservations_never_share_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let a = reserve_round(dir.path(), 1).unwrap();
    let b = reserve_round(dir.path(), 1).unwrap();
    assert_ne!(a.attempt, b.attempt);
}
#[test]
fn gc_abandoned_reservations_are_not_reused() {
    let dir = tempfile::tempdir().unwrap();
    for wanted in 1..=3 {
        assert_eq!(reserve_round(dir.path(), 1).unwrap().attempt, wanted);
    }
}
#[test]
fn gc_legacy_writer_cannot_take_reserved_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let a = reserve_round(dir.path(), 1).unwrap();
    assert!(write_round_record(dir.path(), &set(1, a.attempt, "AC-X")).is_err());
}

fn simultaneous_observations(case: u8) {
    let dir = tempfile::tempdir().unwrap();
    let a = reserve_round(dir.path(), 1).unwrap();
    let b = reserve_round(dir.path(), 1).unwrap();
    let mut first = set(1, a.attempt, "AC-X");
    a.record(dir.path(), &mut first, |record, landing| {
        landing.land(record)
    })
    .unwrap();
    let mut ledger = progress::ProgressLedger::default();
    assert_eq!(ledger.observe_at(dir.path(), &first).unwrap(), 0);
    let mut second = set(1, b.attempt, "AC-X");
    b.record(dir.path(), &mut second, |record, landing| {
        landing.land(record)
    })
    .unwrap();
    assert_eq!(
        ledger.observe_at(dir.path(), &second).unwrap(),
        0,
        "concurrent duplicate counted as a revisit"
    );
    if case == 0 {
        assert_eq!(
            progress::ProgressLedger::load_healing(dir.path())
                .unwrap()
                .ledger,
            ledger
        );
        // Healing must keep the concurrency provenance even if the record
        // and its reservation marker are both lost; the saved copy has it.
        ledger.save(dir.path()).unwrap();
        std::fs::remove_file(
            round_dir(dir.path(), 1).join(format!("attempt-{:02}/reservation.json", first.attempt)),
        )
        .unwrap();
        std::fs::write(
            round_dir(dir.path(), 1).join(attempt_file_name(first.attempt)),
            b"{",
        )
        .unwrap();
        assert_eq!(
            progress::ProgressLedger::load_healing(dir.path())
                .unwrap()
                .ledger,
            ledger
        );
    } else {
        let c = reserve_round(dir.path(), 2).unwrap();
        let third = set(2, c.attempt, if case == 1 { "AC-X" } else { "AC-Y" });
        assert_eq!(
            ledger.observe_at(dir.path(), &third).unwrap(),
            u32::from(case == 1)
        );
    }
}
#[test]
fn gc_simultaneous_duplicates_rebuild_without_a_revisit() {
    simultaneous_observations(0);
}
#[test]
fn gc_a_subsequent_retry_counts_as_a_revisit() {
    simultaneous_observations(1);
}
#[test]
fn gc_a_subsequent_new_state_is_progress() {
    simultaneous_observations(2);
}

#[test]
fn gc_exhausted_frontier_never_merges_sequential_observations() {
    let dir = tempfile::tempdir().unwrap();
    let records = dir.path().join(ACCEPTANCE_RECORDS_DIR);
    std::fs::create_dir_all(&records).unwrap();
    std::fs::write(
        records.join("recording-order.log"),
        "seq=18446744073709551615 1 1 end\n",
    )
    .unwrap();
    assert!(
        reserve_round(dir.path(), 2).is_err(),
        "saturated frontiers silently merged distinct rounds"
    );
}
