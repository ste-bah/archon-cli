//! PLAN-11 review fixes: provisional requests, empty verdicts, fail-closed
//! commits, recorded verdicts and the guards.
use super::*;

/// Item 1: a held change is provisional until its branch landed -- left
/// pending (and never applied) while the branch is unfinished, orphaned when
/// the branch did not land.
#[tokio::test]
async fn a_change_from_a_branch_that_did_not_land_is_never_applied() {
    let w = world();
    w.held_from("scripts/new.sh", b"test -f built\n", None);
    let judge = judge(true);
    let settled = w.settle(Some(&judge)).await;
    assert!(settled.settlements.is_empty() && settled.defects.is_empty());
    assert!(!w.repo.join("scripts/new.sh").exists(), "never applied");
    assert!(judge.seen.lock().unwrap().is_empty(), "never judged");
    // The branch ends refused: the proposal is orphaned, untouched.
    let request = w.held_from(
        "scripts/new.sh",
        b"test -f built\n",
        Some(crate::WorkflowV2Status::NeedsReview),
    );
    let settled = w.settle(Some(&judge)).await;
    let orphaned = settled
        .settlements
        .iter()
        .find(|s| s.request.request_id == request.request_id)
        .unwrap();
    assert_eq!(
        orphaned.resolution.as_ref().unwrap().verdict,
        requests::VERDICT_ORPHANED
    );
    assert!(!w.repo.join("scripts/new.sh").exists());
    assert!(judge.seen.lock().unwrap().is_empty());
}

/// Item 3: a change for a check the contract no longer holds is orphaned:
/// never refuted, never restored, never committed.
#[tokio::test]
async fn an_empty_verdict_never_refutes_restores_or_commits() {
    let w = world();
    std::fs::write(w.repo.join("scripts/check.sh"), "exit 0\n").unwrap();
    let head = git(&w.repo, &["rev-parse", "HEAD"]);
    let other = contract(&[("AC-9", "true")]);
    let ctx = Settle {
        run_root: &w.run,
        roots: Roots {
            repository: &w.repo,
            project: &w.repo,
        },
        store: &w.store,
        contract: &other,
        judge: Some(&judge(false)),
        judge_note: String::new(),
    };
    let settled = settle(&ctx, w.pins.clone()).await;
    let resolution = settled.settlements[0].resolution.clone().unwrap();
    assert_eq!(resolution.verdict, requests::VERDICT_ORPHANED);
    assert!(!resolution.applied);
    assert_eq!(
        std::fs::read(w.repo.join("scripts/check.sh")).unwrap(),
        b"exit 0\n"
    );
    assert_eq!(
        git(&w.repo, &["rev-parse", "HEAD"]),
        head,
        "nothing committed"
    );
}

struct Counting(std::sync::atomic::AtomicUsize, bool);

#[async_trait::async_trait]
impl SourceJudge for Counting {
    async fn judge(&self, _: &SourceJudgeInput) -> Result<SourceVerdict, String> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(SourceVerdict {
            accepted: self.1,
            reason: "judged".into(),
            counterexample: "none".into(),
        })
    }
}

/// Item 11 and the recorded judge: a commit that fails takes the applied
/// change back out and leaves the request pending (failing its check); the
/// next round replays the recorded verdict instead of asking again.
#[tokio::test]
async fn a_failed_commit_is_undone_and_the_recorded_verdict_is_replayed() {
    let w = world();
    w.held("scripts/new.sh", b"test -f built\n");
    let lock = w.repo.join(".git/index.lock");
    std::fs::write(&lock, "").unwrap();
    let first = Counting(Default::default(), true);
    let settled = w.settle(Some(&first)).await;
    assert!(
        settled.defects["AC-2"].contains("taken back out"),
        "{:?}",
        settled.defects
    );
    assert!(!w.repo.join("scripts/new.sh").exists(), "undone");
    std::fs::remove_file(&lock).unwrap();
    // A judge that would refuse is never asked: the recorded verdict stands.
    let second = Counting(Default::default(), false);
    let settled = w.settle(Some(&second)).await;
    assert!(settled.defects.is_empty(), "{:?}", settled.defects);
    assert_eq!(second.0.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert_eq!(
        std::fs::read(w.repo.join("scripts/new.sh")).unwrap(),
        b"test -f built\n"
    );
}

/// Items 7 and 8: a check resting on nothing frozen is a defect; one naming
/// a test nothing defines must not pass.
#[tokio::test]
async fn a_check_on_nothing_frozen_is_a_defect_and_an_unwritten_test_must_fail() {
    let w = world();
    std::fs::write(w.repo.join("Cargo.toml"), "[package]\nname = \"p\"\n").unwrap();
    let c = contract(&[
        ("AC-1", "mytool --verify"),
        ("AC-2", "cargo test not_written -- --exact"),
        ("AC-3", "test -f built"),
    ]);
    let roots = Roots {
        repository: &w.repo,
        project: &w.repo,
    };
    let pins = pin_contract(&c, "d", &roots, ORIGIN_FREEZE, &w.store.blobs);
    let ctx = Settle {
        run_root: &w.run,
        roots,
        store: &w.store,
        contract: &c,
        judge: None,
        judge_note: "none".into(),
    };
    let settled = settle(&ctx, pins).await;
    assert!(
        settled.defects["AC-1"].contains("rest on nothing frozen"),
        "{:?}",
        settled.defects
    );
    assert!(
        !settled.defects.contains_key("AC-3"),
        "inline logic is frozen with the contract"
    );
    assert!(
        settled.must_fail["AC-2"].contains("not_written"),
        "{:?}",
        settled.must_fail
    );
}

/// Item 11: an unreadable request store is the round's error, never skipped.
#[tokio::test]
async fn an_unreadable_request_record_is_an_error_not_a_skip() {
    let w = world();
    let dir = requests::requests_dir(&w.run);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("csr-broken.json"), "not json").unwrap();
    let settled = w.settle(Some(&judge(true))).await;
    assert!(
        settled.errors[0].contains("unreadable"),
        "{:?}",
        settled.errors
    );
}
