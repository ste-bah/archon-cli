//! Issue 338: a re-pin is computed over the pins the store binds when it is
//! written -- never written over pins a settled publish or another repin
//! changed under it -- and a publish no settlement can settle pauses the
//! round instead of leaving the change pending.
use super::*;
use crate::check_source_pins::RepinLink;

/// Pins another writer stored after the round read `w.pins`.
fn moved_on(w: &World, acceptance_digest: &str) -> CheckSourcePins {
    let mut other = w.pins.clone();
    other.acceptance_digest = acceptance_digest.into();
    other.repins.push(RepinLink {
        request_id: "csr-another-round".into(),
        check_ids: BTreeSet::from(["AC-1".to_string()]),
        root: SourceRoot::Repository,
        path: "scripts/check.sh".into(),
        item: None,
        from: None,
        to: None,
        reason: "settled elsewhere".into(),
        at: "2026-10-06T00:00:00Z".into(),
        prior_digest: "p".into(),
    });
    w.store.write(&other).unwrap();
    other
}

#[tokio::test]
async fn a_repin_over_pins_another_repin_changed_keeps_both_repins() {
    let w = world();
    moved_on(&w, &w.pins.acceptance_digest);
    let request = w.held("scripts/new.sh", b"test -f built\n");
    let settled = w.settle(Some(&judge(true))).await;
    assert!(settled.defects.is_empty(), "{:?}", settled.defects);
    let stored = w.store.read().unwrap().unwrap();
    let links: Vec<&str> = stored
        .repins
        .iter()
        .map(|l| l.request_id.as_str())
        .collect();
    assert_eq!(
        links,
        ["csr-another-round", request.request_id.as_str()],
        "a repin was overwritten"
    );
    assert_eq!(stored, settled.pins, "the round goes on with what it wrote");
    assert_eq!(
        stored.checks["AC-2"].sources[0].digest,
        request.proposed_digest
    );
}

#[tokio::test]
async fn a_repin_over_a_set_republished_under_it_writes_nothing_and_stays_pending() {
    let w = world();
    let republished = moved_on(&w, "republished");
    let request = w.held("scripts/new.sh", b"test -f built\n");
    let settled = w.settle(Some(&judge(true))).await;
    assert_eq!(w.store.read().unwrap().unwrap(), republished, "overwritten");
    assert!(settled.settlements[0].resolution.is_none(), "{settled:?}");
    let why = &settled.defects["AC-2"];
    assert!(
        why.contains("republished while the change was settled"),
        "{why}"
    );
    assert!(settled.paused.is_none(), "a republish is no stall");
    assert_eq!(
        requests::pending(&w.run).unwrap()[0].request_id,
        request.request_id
    );
}

#[tokio::test]
async fn a_repin_over_a_journal_no_settlement_can_settle_pauses_the_round() {
    let mut w = world();
    // The frozen sidecar of a set whose publish left its journal; no host is
    // linked into these tests, so nothing can settle it.
    let pin = w.run.with_file_name("pins/set.json");
    std::fs::create_dir_all(pin.parent().unwrap()).unwrap();
    w.store.pin = Some(pin.clone());
    w.store.tasks_root = Some(w.repo.clone());
    let journal = crate::task_set_publish_lock::journal_paths(&pin)[0].clone();
    std::fs::write(&journal, br#"{"state": "committed"}"#).unwrap();
    w.held("scripts/new.sh", b"test -f built\n");
    w.held("scripts/check.sh", b"test -f built && exit 0\n");
    let judge = judge(true);
    let settled = w.settle(Some(&judge)).await;
    let evidence = settled.paused.as_deref().expect("the round pauses");
    assert!(evidence.contains("state: committed"), "{evidence}");
    assert!(evidence.contains("Operator remedy"), "{evidence}");
    assert_eq!(judge.seen.lock().unwrap().len(), 1, "settling went on");
    assert_eq!(
        w.store.read().unwrap().unwrap(),
        w.pins,
        "re-pinned over it"
    );
    assert!(journal.exists());
    assert_eq!(requests::pending(&w.run).unwrap().len(), 2);
}
