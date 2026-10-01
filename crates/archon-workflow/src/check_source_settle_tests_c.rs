//! PLAN-11 review minors: idempotent re-settlement, reopened proposals,
//! oversize sources, links, the repository lock and git failures.
use super::*;

/// Minor 2: an accepted change whose settlement record was lost (a crash
/// after the commit) is settled again as accepted -- never called stale --
/// and re-pinned once.
#[tokio::test]
async fn an_applied_change_settled_again_after_a_crash_is_not_stale() {
    let w = world();
    let request = w.held("scripts/new.sh", b"test -f built\n");
    let settled = w.settle(Some(&judge(true))).await;
    assert_eq!(settled.pins.repins.len(), 1);
    let record =
        requests::requests_dir(&w.run).join(format!("{}.resolved.json", request.request_id));
    std::fs::remove_file(record).unwrap();
    let ctx_pins = w.store.read().unwrap().unwrap();
    let ctx = Settle {
        run_root: &w.run,
        roots: Roots {
            repository: &w.repo,
            project: &w.repo,
        },
        store: &w.store,
        contract: &w.contract,
        judge: Some(&judge(true)),
        judge_note: String::new(),
    };
    let again = settle(&ctx, ctx_pins).await;
    let resolution = again.settlements[0].resolution.clone().unwrap();
    assert_eq!(resolution.verdict, VERDICT_ACCEPTED, "{resolution:?}");
    assert_eq!(again.pins.repins.len(), 1, "re-pinned once");
    assert!(again.defects.is_empty(), "{:?}", again.defects);
}

/// Minor 3: a refused change made again with the same bytes is a new
/// request, judged again -- never stuck on the settled one.
#[tokio::test]
async fn a_refused_drift_that_reappears_is_judged_again() {
    let w = world();
    std::fs::write(w.repo.join("scripts/check.sh"), "exit 0\n").unwrap();
    let first = w.settle(Some(&judge(false))).await;
    assert!(first.defects.is_empty(), "{:?}", first.defects);
    std::fs::write(w.repo.join("scripts/check.sh"), "exit 0\n").unwrap();
    let second = w.settle(Some(&judge(false))).await;
    assert!(second.defects.is_empty(), "{:?}", second.defects);
    assert_eq!(second.settlements.len(), 1);
    assert_ne!(
        first.settlements[0].request.request_id,
        second.settlements[0].request.request_id
    );
    assert_eq!(
        std::fs::read(w.repo.join("scripts/check.sh")).unwrap(),
        b"test -f built || exit 1\n"
    );
}

/// Minor 4: a source larger than the judge is shown whole is judged by its
/// change, in parts -- never left pending.
#[tokio::test]
async fn an_oversize_source_is_judged_by_its_change_in_parts() {
    let w = world();
    let big: String = (0..40_000).map(|n| format!("echo line {n}\n")).collect();
    assert!(big.len() > MAX_JUDGED_SOURCE_BYTES);
    std::fs::write(w.repo.join("scripts/check.sh"), &big).unwrap();
    let judge = judge(true);
    let settled = w.settle(Some(&judge)).await;
    assert!(settled.defects.is_empty(), "{:?}", settled.defects);
    let seen = judge.seen.lock().unwrap();
    assert!(seen.len() > 1, "judged in parts");
    assert!(
        seen.iter()
            .all(|input| input.diff.is_some() && input.proposed.is_none())
    );
    assert_eq!(
        settled.settlements[0].resolution.as_ref().unwrap().verdict,
        VERDICT_ACCEPTED
    );
}

/// Minor 7: a pinned file swapped for a link -- even to identical bytes --
/// is a change; it is refused without judging and the file put back.
#[tokio::test]
async fn a_pinned_file_swapped_for_a_link_is_a_change_and_is_restored() {
    let w = world();
    let script = w.repo.join("scripts/check.sh");
    let twin = w.repo.join("scripts/twin.sh");
    std::fs::copy(&script, &twin).unwrap();
    std::fs::remove_file(&script).unwrap();
    std::os::unix::fs::symlink("twin.sh", &script).unwrap();
    let judge = judge(true);
    let settled = w.settle(Some(&judge)).await;
    assert!(
        judge.seen.lock().unwrap().is_empty(),
        "never judged: {:?} {:?}",
        judge
            .seen
            .lock()
            .unwrap()
            .iter()
            .map(|i| i.source.clone())
            .collect::<Vec<_>>(),
        settled
            .settlements
            .iter()
            .map(|s| (s.request.label(), s.resolution.clone()))
            .collect::<Vec<_>>()
    );
    let resolution = settled.settlements[0].resolution.clone().unwrap();
    assert_eq!(resolution.verdict, VERDICT_REFUTED);
    let meta = std::fs::symlink_metadata(&script).unwrap();
    assert!(
        meta.file_type().is_file(),
        "the link was replaced by the pinned file"
    );
    assert_eq!(
        std::fs::read(&twin).unwrap(),
        b"test -f built || exit 1\n",
        "never written through"
    );
}

/// Minor 9: an accepted change is written only under the repository's
/// write lock.
#[test]
fn an_accepted_write_waits_for_the_repository_lock() {
    let w = world();
    w.held("scripts/new.sh", b"test -f built\n");
    let target = w.repo.join("scripts/new.sh");
    let repo = w.repo.clone();
    let (held_tx, held_rx) = std::sync::mpsc::channel::<()>();
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let holder = std::thread::spawn(move || {
        crate::write_coordinator::patch_apply::with_repo_lock(&repo, || {
            held_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            Ok(())
        })
        .unwrap();
    });
    held_rx.recv().unwrap();
    let settler = std::thread::spawn(move || {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let judge = judge(true);
        let settled = runtime.block_on(w.settle(Some(&judge)));
        (settled, w)
    });
    std::thread::sleep(std::time::Duration::from_millis(1500));
    assert!(
        !target.exists(),
        "written while another holder had the lock"
    );
    release_tx.send(()).unwrap();
    holder.join().unwrap();
    // The world comes back: its directory must outlive the checks below.
    let (settled, _world) = settler.join().unwrap();
    assert!(settled.defects.is_empty(), "{:?}", settled.defects);
    assert!(
        target.exists(),
        "{:?}",
        settled
            .settlements
            .iter()
            .map(|s| (s.request.label(), s.note.clone()))
            .collect::<Vec<_>>()
    );
}

/// Minor 10: a git failure is the commit's failure: the change is taken
/// back out, never left applied and uncommitted.
#[tokio::test]
async fn a_git_failure_is_never_a_commit() {
    let w = world();
    w.held("scripts/new.sh", b"test -f built\n");
    std::fs::write(w.repo.join(".git/index"), b"garbage").unwrap();
    let settled = w.settle(Some(&judge(true))).await;
    assert!(
        settled.defects["AC-2"].contains("taken back out"),
        "{:?}",
        settled.defects
    );
    assert!(!w.repo.join("scripts/new.sh").exists());
}
