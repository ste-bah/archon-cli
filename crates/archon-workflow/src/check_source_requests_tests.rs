use super::*;

fn new<'a>(path: &'a str, proposed: &'a [u8], branch: &'a str) -> NewRequest<'a> {
    NewRequest {
        origin: ORIGIN_LANDING,
        check_ids: BTreeSet::from(["AC-1".to_string()]),
        root: SourceRoot::Repository,
        path,
        item: None,
        was_pinned: true,
        pinned_digest: Some(content_digest(b"pinned")),
        proposed: Some(proposed),
        proposed_file: None,
        landed_file_digest: None,
        call_id: "wave",
        branch_id: branch,
        task_ids: vec!["TASK-1".into()],
    }
}

#[test]
fn a_request_is_recorded_once_with_its_bytes_and_settled_once() {
    let dir = tempfile::tempdir().unwrap();
    let run = dir.path();
    let first = record(run, new("tests/it.rs", b"proposed", "wave-0")).unwrap();
    let again = record(run, new("tests/it.rs", b"proposed", "wave-0")).unwrap();
    assert_eq!(first, again, "a resumed branch records nothing twice");
    let mut from_another_task = new("tests/it.rs", b"other", "wave-0");
    from_another_task.task_ids = vec!["TASK-2".into()];
    let other = record(run, from_another_task).unwrap();
    assert_ne!(first.request_id, other.request_id);
    assert_eq!(
        blobs(run)
            .get(first.proposed_digest.as_ref().unwrap())
            .unwrap(),
        b"proposed"
    );
    assert_eq!(pending(run).unwrap().len(), 2);
    let refused = RequestResolution {
        request_id: first.request_id.clone(),
        verdict: VERDICT_REFUTED.into(),
        reason: "drops the assertion".into(),
        counterexample: String::new(),
        applied: false,
        repinned: false,
        at: "2026-01-01T00:00:00Z".into(),
    };
    settle_record(run, &refused).unwrap();
    let mut second = refused.clone();
    second.verdict = VERDICT_ACCEPTED.into();
    settle_record(run, &second).unwrap();
    let settled = all(run).unwrap();
    let (_, resolution) = settled
        .iter()
        .find(|(request, _)| request.request_id == first.request_id)
        .unwrap();
    assert_eq!(
        resolution.as_ref().unwrap().verdict,
        VERDICT_REFUTED,
        "write-once"
    );
    assert_eq!(pending(run).unwrap(), vec![other]);
    assert!(
        last_refusal(run, "tests/it.rs")
            .unwrap()
            .contains("drops the assertion")
    );
    assert!(last_refusal(run, "tests/other.rs").is_none());
}

/// Item 9: a newer proposal supersedes the older pending ones for the same
/// source from the same task -- and only those.
#[test]
fn a_newer_proposal_supersedes_older_ones_for_the_same_source_and_task() {
    let dir = tempfile::tempdir().unwrap();
    let run = dir.path();
    let older = record(run, new("tests/it.rs", b"one", "wave-0")).unwrap();
    let elsewhere = record(run, new("tests/other.rs", b"one", "wave-0")).unwrap();
    let mut other_task = new("tests/it.rs", b"two", "wave-1");
    other_task.task_ids = vec!["TASK-2".into()];
    let other_task = record(run, other_task).unwrap();
    let newer = record(run, new("tests/it.rs", b"three", "wave-2")).unwrap();
    let pending: Vec<String> = pending(run)
        .unwrap()
        .into_iter()
        .map(|r| r.request_id)
        .collect();
    assert!(!pending.contains(&older.request_id), "{pending:?}");
    for kept in [&elsewhere, &other_task, &newer] {
        assert!(pending.contains(&kept.request_id), "{pending:?}");
    }
    let (_, resolution) = all(run)
        .unwrap()
        .into_iter()
        .find(|(r, _)| r.request_id == older.request_id)
        .unwrap();
    assert_eq!(resolution.unwrap().verdict, VERDICT_SUPERSEDED);
}

/// Item 10: the held-change gap and the unreadable-pins gap are the host's:
/// the residual planner never plans them as work.
#[test]
fn the_held_change_gap_is_host_owned() {
    use crate::v2::script::residual_plan::host_environment_gap;
    assert!(host_environment_gap(
        &format!("{CHECK_SOURCE_HELD_GAP_PREFIX}w-0"),
        "held"
    ));
    assert!(host_environment_gap(
        &format!("{CHECK_SOURCE_PINS_UNAVAILABLE_GAP_PREFIX}w-0"),
        "x"
    ));
    assert!(!host_environment_gap(
        "missing_tests_w-0",
        "write the tests"
    ));
}
