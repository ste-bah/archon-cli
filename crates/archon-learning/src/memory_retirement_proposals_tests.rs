use super::*;

fn test_db() -> crate::cozo_guard::TestDb {
    crate::cozo_guard::test_sqlite_db("test-memory-retirement-proposals")
}

fn proposal(memory_id: &str, reason: &str) -> MemoryRetirementProposalRecord {
    MemoryRetirementProposalRecord {
        proposal_id: MemoryRetirementProposalRecord::stable_id(memory_id, reason),
        memory_id: memory_id.to_string(),
        memory_title: "a title".to_string(),
        excerpt: "something the user told Archon months ago".to_string(),
        memory_type: "fact".to_string(),
        importance: 0.08,
        reason_kind: reason.to_string(),
        reason_detail: "91 days since access, floor 0.30".to_string(),
        run_id: "run-1".to_string(),
        status: MemoryRetirementStatus::Pending,
        created_at: "2026-08-10T03:00:00Z".to_string(),
    }
}

#[test]
fn a_proposal_round_trips() {
    let db = test_db();
    let record = proposal("mem-1", "stale");

    insert_memory_retirement_proposal(&db, &record).expect("insert");
    let restored = get_memory_retirement_proposal(&db, &record.proposal_id)
        .expect("get")
        .expect("present");

    assert_eq!(restored, record);
}

#[test]
fn re_proposing_the_same_memory_does_not_pile_up_duplicates() {
    // A nightly pass re-derives the same candidates from the same store every
    // night. Without a stable id it would hand a reviewer one row per night for
    // a memory nobody has touched.
    let db = test_db();
    insert_memory_retirement_proposal(&db, &proposal("mem-1", "stale")).expect("first");
    insert_memory_retirement_proposal(
        &db,
        &MemoryRetirementProposalRecord {
            run_id: "run-2".to_string(),
            ..proposal("mem-1", "stale")
        },
    )
    .expect("second");

    let pending = list_memory_retirement_proposals(&db, MemoryRetirementStatus::Pending)
        .expect("list pending");

    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].run_id, "run-2", "the later pass's evidence wins");
}

#[test]
fn the_same_memory_under_two_rules_is_two_proposals() {
    // Stale and overflow are different arguments for removing a memory, and a
    // reviewer refusing one is not refusing the other.
    let db = test_db();
    insert_memory_retirement_proposal(&db, &proposal("mem-1", "stale")).expect("stale");
    insert_memory_retirement_proposal(&db, &proposal("mem-1", "overflow")).expect("overflow");

    let pending = list_memory_retirement_proposals(&db, MemoryRetirementStatus::Pending)
        .expect("list pending");

    assert_eq!(pending.len(), 2);
}

#[test]
fn a_decision_moves_the_row_out_of_pending() {
    let db = test_db();
    let record = proposal("mem-1", "stale");
    insert_memory_retirement_proposal(&db, &record).expect("insert");

    let decided = decide_memory_retirement_proposal(
        &db,
        &record.proposal_id,
        MemoryRetirementStatus::Rejected,
    )
    .expect("decide");

    assert_eq!(decided.status, MemoryRetirementStatus::Rejected);
    assert!(
        list_memory_retirement_proposals(&db, MemoryRetirementStatus::Pending)
            .expect("list")
            .is_empty()
    );
    assert_eq!(
        list_memory_retirement_proposals(&db, MemoryRetirementStatus::Rejected)
            .expect("list")
            .len(),
        1
    );
}

#[test]
fn a_decision_cannot_be_overwritten() {
    // Keeping refusals is the point of not deleting rejected rows. A second call
    // silently turning a rejection into an approval would erase exactly the fact
    // the record exists to hold.
    let db = test_db();
    let record = proposal("mem-1", "stale");
    insert_memory_retirement_proposal(&db, &record).expect("insert");
    decide_memory_retirement_proposal(&db, &record.proposal_id, MemoryRetirementStatus::Rejected)
        .expect("first decision");

    let second = decide_memory_retirement_proposal(
        &db,
        &record.proposal_id,
        MemoryRetirementStatus::Approved,
    );

    assert!(second.is_err(), "a decided proposal must not be re-decided");
    assert_eq!(
        get_memory_retirement_proposal(&db, &record.proposal_id)
            .expect("get")
            .expect("present")
            .status,
        MemoryRetirementStatus::Rejected
    );
}

#[test]
fn pending_is_not_a_decision() {
    let db = test_db();
    let record = proposal("mem-1", "stale");
    insert_memory_retirement_proposal(&db, &record).expect("insert");

    assert!(
        decide_memory_retirement_proposal(
            &db,
            &record.proposal_id,
            MemoryRetirementStatus::Pending
        )
        .is_err()
    );
}

#[test]
fn an_unknown_stored_status_reads_as_pending() {
    // A row written by a future build must never be readable as consent to
    // delete. Defaulting to the inert status is the only safe direction.
    assert_eq!(
        MemoryRetirementStatus::from_stored("SomethingNewerWrote"),
        MemoryRetirementStatus::Pending
    );
    assert_eq!(
        MemoryRetirementStatus::from_stored("Approved"),
        MemoryRetirementStatus::Approved
    );
}

#[test]
fn deciding_a_proposal_that_does_not_exist_is_an_error() {
    let db = test_db();

    assert!(
        decide_memory_retirement_proposal(&db, "mrp-nope", MemoryRetirementStatus::Approved)
            .is_err()
    );
}
