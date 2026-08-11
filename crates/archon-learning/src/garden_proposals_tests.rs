use super::*;

fn test_db() -> crate::cozo_guard::TestDb {
    crate::cozo_guard::test_sqlite_db("test-garden-proposals")
}

fn proposal(kind: GardenProposalKind, subject: &str) -> GardenProposalRecord {
    GardenProposalRecord {
        proposal_id: GardenProposalRecord::stable_id(kind, subject),
        proposal_kind: kind,
        subject_id: subject.to_string(),
        subject_title: "a title".to_string(),
        excerpt: "something recorded months ago".to_string(),
        detail: "untouched for 91 days, below the importance floor".to_string(),
        payload_json: "{}".to_string(),
        run_id: "run-1".to_string(),
        status: GardenProposalStatus::Pending,
        applied_ref: String::new(),
        created_at: "2026-08-10T03:00:00Z".to_string(),
        decided_at: String::new(),
    }
}

#[test]
fn a_proposal_round_trips_with_its_kind() {
    let db = test_db();
    let record = proposal(GardenProposalKind::SemanticConsolidation, "cand-1");

    insert_garden_proposal(&db, &record).expect("insert");

    let restored = get_garden_proposal(&db, &record.proposal_id)
        .expect("get")
        .expect("present");
    assert_eq!(restored, record);
}

#[test]
fn the_three_kinds_do_not_collide_on_one_subject() {
    // A memory and a rule could share an id shape; the kind is part of the
    // identity so one decision never lands on the other's subject.
    let db = test_db();
    for kind in [
        GardenProposalKind::MemoryRetirement,
        GardenProposalKind::RuleRetirement,
        GardenProposalKind::SemanticConsolidation,
    ] {
        insert_garden_proposal(&db, &proposal(kind, "subject-1")).expect("insert");
    }

    assert_eq!(
        list_garden_proposals(&db, GardenProposalStatus::Pending)
            .expect("list")
            .len(),
        3
    );
}

#[test]
fn the_full_lifecycle_runs_approve_apply_rollback() {
    let db = test_db();
    let record = proposal(GardenProposalKind::MemoryRetirement, "mem-1");
    insert_garden_proposal(&db, &record).expect("insert");

    let approved = transition_garden_proposal(
        &db,
        &record.proposal_id,
        GardenProposalStatus::Approved,
        None,
        "t1",
    )
    .expect("approve");
    assert_eq!(approved.status, GardenProposalStatus::Approved);

    let applied = transition_garden_proposal(
        &db,
        &record.proposal_id,
        GardenProposalStatus::Applied,
        Some("mem-1"),
        "t2",
    )
    .expect("apply");
    assert_eq!(applied.status, GardenProposalStatus::Applied);
    assert_eq!(
        applied.applied_ref, "mem-1",
        "a rollback needs to know what the apply step touched"
    );

    let rolled_back = transition_garden_proposal(
        &db,
        &record.proposal_id,
        GardenProposalStatus::RolledBack,
        None,
        "t3",
    )
    .expect("rollback");
    assert_eq!(rolled_back.status, GardenProposalStatus::RolledBack);
    assert_eq!(
        rolled_back.applied_ref, "mem-1",
        "the rollback must not erase what it undid"
    );
}

#[test]
fn a_pending_proposal_cannot_be_applied_without_approval() {
    // The single most important assertion here: there is no sequence of
    // automatic steps from raised to applied.
    let db = test_db();
    let record = proposal(GardenProposalKind::MemoryRetirement, "mem-1");
    insert_garden_proposal(&db, &record).expect("insert");

    assert!(
        transition_garden_proposal(
            &db,
            &record.proposal_id,
            GardenProposalStatus::Applied,
            Some("mem-1"),
            "t1",
        )
        .is_err(),
        "an unapproved proposal was applied"
    );
    assert_eq!(
        get_garden_proposal(&db, &record.proposal_id)
            .expect("get")
            .expect("present")
            .status,
        GardenProposalStatus::Pending
    );
}

#[test]
fn a_rejected_proposal_cannot_be_revived() {
    let db = test_db();
    let record = proposal(GardenProposalKind::RuleRetirement, "rule-1");
    insert_garden_proposal(&db, &record).expect("insert");
    transition_garden_proposal(
        &db,
        &record.proposal_id,
        GardenProposalStatus::Rejected,
        None,
        "t1",
    )
    .expect("reject");

    assert!(
        transition_garden_proposal(
            &db,
            &record.proposal_id,
            GardenProposalStatus::Approved,
            None,
            "t2",
        )
        .is_err()
    );
}

#[test]
fn a_rolled_back_proposal_cannot_be_applied_again() {
    // Re-applying would take a memory the reviewer restored and hide it again,
    // with the original approval standing in for a decision nobody made twice.
    let db = test_db();
    let record = proposal(GardenProposalKind::MemoryRetirement, "mem-1");
    insert_garden_proposal(&db, &record).expect("insert");
    for next in [
        GardenProposalStatus::Approved,
        GardenProposalStatus::Applied,
        GardenProposalStatus::RolledBack,
    ] {
        transition_garden_proposal(&db, &record.proposal_id, next, Some("mem-1"), "t")
            .expect("transition");
    }

    assert!(
        transition_garden_proposal(
            &db,
            &record.proposal_id,
            GardenProposalStatus::Applied,
            Some("mem-1"),
            "t",
        )
        .is_err()
    );
}

#[test]
fn raising_a_proposal_again_does_not_reopen_a_decided_one() {
    // A nightly pass re-derives the same candidates. Overwriting a rejection
    // with a fresh Pending row would offer the same thing every night until the
    // reviewer approved it just to make it stop.
    let db = test_db();
    let record = proposal(GardenProposalKind::MemoryRetirement, "mem-1");
    raise_garden_proposal(&db, &record).expect("raise");
    transition_garden_proposal(
        &db,
        &record.proposal_id,
        GardenProposalStatus::Rejected,
        None,
        "t1",
    )
    .expect("reject");

    let re_raised = raise_garden_proposal(
        &db,
        &GardenProposalRecord {
            run_id: "run-2".to_string(),
            ..record.clone()
        },
    )
    .expect("re-raise");

    assert_eq!(re_raised.status, GardenProposalStatus::Rejected);
    assert!(
        list_garden_proposals(&db, GardenProposalStatus::Pending)
            .expect("list")
            .is_empty(),
        "a refused proposal came back as pending"
    );
}

#[test]
fn re_raising_a_still_pending_proposal_refreshes_its_evidence() {
    let db = test_db();
    let record = proposal(GardenProposalKind::MemoryRetirement, "mem-1");
    raise_garden_proposal(&db, &record).expect("raise");

    raise_garden_proposal(
        &db,
        &GardenProposalRecord {
            run_id: "run-2".to_string(),
            detail: "untouched for 98 days".to_string(),
            ..record.clone()
        },
    )
    .expect("re-raise");

    let pending = list_garden_proposals(&db, GardenProposalStatus::Pending).expect("list");
    assert_eq!(pending.len(), 1, "no duplicate row");
    assert_eq!(pending[0].run_id, "run-2");
    assert_eq!(pending[0].detail, "untouched for 98 days");
}

#[test]
fn an_unknown_stored_status_reads_as_pending() {
    assert_eq!(
        GardenProposalStatus::from_stored("SomethingNewerWrote"),
        GardenProposalStatus::Pending,
        "an unrecognised status must never authorise a change"
    );
}

#[test]
fn an_unknown_stored_kind_is_skipped_rather_than_guessed() {
    // The kind decides which applier acts. Guessing would point one applier at
    // another's subject.
    assert!(GardenProposalKind::from_stored("something_new").is_none());
}

#[test]
fn the_lifecycle_forbids_every_transition_it_does_not_name() {
    use GardenProposalStatus::*;
    let allowed = [
        (Pending, Approved),
        (Pending, Rejected),
        (Approved, Applied),
        (Applied, RolledBack),
    ];
    for from in [Pending, Approved, Rejected, Applied, RolledBack] {
        for to in [Pending, Approved, Rejected, Applied, RolledBack] {
            assert_eq!(
                from.may_transition_to(to),
                allowed.contains(&(from, to)),
                "{} -> {} is not classified as intended",
                from.as_str(),
                to.as_str()
            );
        }
    }
}

#[test]
fn deciding_a_proposal_that_does_not_exist_is_an_error() {
    let db = test_db();

    assert!(
        transition_garden_proposal(&db, "gp-nope", GardenProposalStatus::Approved, None, "t")
            .is_err()
    );
}
