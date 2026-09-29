//! Batch J2: the regression search is a signature-aware bisection. A point
//! is BAD when a check fails there with the signature it fails with now,
//! GOOD when it passes or fails another way; a landing that did not build
//! gives no verdict. A group sharing a signature is confirmed member by
//! member at the break.
//!
//! The shape of wf-0ddadd81's attempt 6: AC-AHDM-003 (TASK-014),
//! AC-DL-005 (TASK-010) and AC-DL-006 (TASK-003) all end in
//! `Error: unknown asset_class `unknown``. At the run base the feature did
//! not exist (the checks fail there, but another way); the group was green
//! at a TASK-010 landing, and a TASK-009 landing broke it.

use std::collections::BTreeMap;

use super::tests::World;
use super::{CheckObserver, FailingCheck, SearchBudget, Verdict, failure_signature};
use crate::write_coordinator::worktree_isolation::run_git;

const SIGNATURE: &str =
    "warning: x\nError: unknown asset_class `unknown`: expected one of [\"future\"]\n";

fn member(id: &str, owner: &str) -> FailingCheck {
    FailingCheck {
        id: id.into(),
        owners: vec![owner.into()],
        signature: failure_signature(Some(1), SIGNATURE, ""),
        fingerprint: "f1".into(),
    }
}

/// `flags/<id>` at the commit: `ok` passes; `bad` fails as at the tip; no
/// flag fails another way (the command does not exist yet); anything else
/// did not build.
struct SignedObserver {
    repo: std::path::PathBuf,
    seen: std::sync::Mutex<Vec<(String, Vec<String>)>>,
}

impl SignedObserver {
    fn new(world: &World) -> Self {
        Self {
            repo: world.repo.clone(),
            seen: Default::default(),
        }
    }

    fn seen(&self) -> Vec<(String, Vec<String>)> {
        self.seen.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl CheckObserver for SignedObserver {
    async fn observe(&self, commit: &str, ids: &[String]) -> Option<BTreeMap<String, Verdict>> {
        self.seen
            .lock()
            .unwrap()
            .push((commit.to_string(), ids.to_vec()));
        Some(
            ids.iter()
                .map(|id| {
                    let flag = run_git(&["show", &format!("{commit}:flags/{id}")], &self.repo)
                        .ok()
                        .filter(|out| out.status.success())
                        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string());
                    let failed =
                        |exit, text: &str| Verdict::Failed(failure_signature(exit, text, ""));
                    let verdict = match flag.as_deref() {
                        Some("ok") => Verdict::Held(true),
                        Some("bad") => failed(Some(1), SIGNATURE),
                        None => failed(Some(2), "error: unrecognized subcommand 'ingest'\n"),
                        Some(_) => failed(Some(101), "error: could not compile `app`\n"),
                    };
                    (id.clone(), verdict)
                })
                .collect(),
        )
    }
}

/// The base fails another way (the feature is absent): GOOD, so the
/// bisection runs from it and pins the break for the whole group.
#[tokio::test]
async fn a_group_failing_another_way_at_the_base_is_bisected_to_its_break() {
    let w = World::new(&[]);
    w.step("flags/c", "ok", "remediate-task-003-1", "TASK-003");
    w.step("flags/a", "ok", "remediate-task-014-2", "TASK-014");
    let green = w.step("flags/b", "ok", "review-remediate-task-010-3", "TASK-010");
    // The break: TASK-009 tightens the shared parser.
    std::fs::write(w.repo.join("flags/a"), "bad").unwrap();
    std::fs::write(w.repo.join("flags/c"), "bad").unwrap();
    let breaking = w.step("flags/b", "bad", "review-remediate-task-009-4", "TASK-009");
    w.step("pad.txt", "1", "review-remediate-task-012-5", "TASK-012");
    w.step("x.txt", "1", "review-remediate-task-014-6", "TASK-014");
    w.step("x.txt", "2", "review-remediate-cross-task-7", "TASK-010");
    w.step("pad.txt", "2", "review-remediate-task-015-8", "TASK-015");
    let failing = [
        member("a", "TASK-014"),
        member("b", "TASK-010"),
        member("c", "TASK-003"),
    ];
    let observer = SignedObserver::new(&w);
    let found = w
        .attribute(&failing, &observer, SearchBudget::default())
        .await;
    for id in ["a", "b", "c"] {
        let regression = (found.regressions.get(id)).unwrap_or_else(|| panic!("{id}: {found:#?}"));
        assert_eq!(regression.landing_commit, breaking, "{id}");
        assert_eq!(regression.held_at, green, "{id}");
        assert_eq!(regression.tasks, ["TASK-009"], "{id}");
        assert_eq!(
            regression.changed_files,
            ["flags/a", "flags/b", "flags/c"],
            "{id}"
        );
    }
    assert_eq!(found.regressions["a"].probed_as, None);
    assert_eq!(found.regressions["b"].probed_as.as_deref(), Some("a"));
    // The base once for all, a bisection for the lead, two confirmations.
    let seen = observer.seen();
    assert_eq!(seen[0].1, ["a", "b", "c"], "{seen:?}");
    assert!(seen.len() <= 6, "{seen:?}");
}

/// A member whose own feature landed already broken fails first at its
/// owner's landing: it is not handed the group's break, and the others are
/// split off and pinned at theirs.
#[tokio::test]
async fn a_member_that_never_passed_is_pinned_where_it_first_failed_so() {
    let w = World::new(&[]);
    w.step("flags/c", "ok", "remediate-task-003-1", "TASK-003");
    w.step("flags/b", "ok", "review-remediate-task-010-2", "TASK-010");
    std::fs::write(w.repo.join("flags/c"), "bad").unwrap();
    let breaking = w.step("flags/b", "bad", "review-remediate-task-009-3", "TASK-009");
    let own = w.step("flags/a", "bad", "review-remediate-task-014-4", "TASK-014");
    w.step("pad.txt", "1", "review-remediate-task-015-5", "TASK-015");
    let failing = [
        member("a", "TASK-014"),
        member("b", "TASK-010"),
        member("c", "TASK-003"),
    ];
    let observer = SignedObserver::new(&w);
    let found = w
        .attribute(&failing, &observer, SearchBudget::default())
        .await;
    assert_eq!(found.regressions["a"].landing_commit, own, "{found:#?}");
    for id in ["b", "c"] {
        assert_eq!(found.regressions[id].landing_commit, breaking, "{id}");
        assert_eq!(found.regressions[id].tasks, ["TASK-009"], "{id}");
    }
}

/// A landing between the green one and the break that did not build (the
/// next landing fixed that) is never pinned as the break.
#[tokio::test]
async fn a_landing_that_did_not_build_is_never_pinned_as_the_break() {
    let w = World::new(&[]);
    w.step("flags/b", "ok", "implement-task-010-1", "TASK-010");
    w.step("pad.txt", "2", "implement-pad-2", "TASK-PAD");
    w.step("pad.txt", "3", "implement-pad-3", "TASK-PAD");
    let unbuildable = w.step("flags/b", "nocompile", "implement-x-4", "TASK-X");
    w.step("flags/b", "ok", "implement-y-5", "TASK-Y");
    let breaking = w.step("flags/b", "bad", "review-remediate-task-009-6", "TASK-009");
    w.step("pad.txt", "7", "implement-pad-7", "TASK-PAD");
    let observer = SignedObserver::new(&w);
    let found = w
        .attribute(
            &[member("b", "TASK-010")],
            &observer,
            SearchBudget::default(),
        )
        .await;
    let regression = &found.regressions["b"];
    assert_ne!(regression.landing_commit, unbuildable, "{found:#?}");
    assert_eq!(regression.landing_commit, breaking, "{found:#?}");
}

/// Thirty landings: the break is found in about log2 of them, never by
/// walking the owner's landings one by one.
#[tokio::test]
async fn the_break_is_found_in_about_log2_probes() {
    let w = World::new(&[]);
    w.step("flags/b", "ok", "implement-task-010-0", "TASK-010");
    let mut breaking = String::new();
    for at in 1..30 {
        let (file, body, task) = match at {
            17 => ("flags/b", "bad".to_string(), "TASK-009"),
            _ => ("pad.txt", at.to_string(), "TASK-010"),
        };
        let sha = w.step(file, &body, &format!("review-remediate-{at}"), task);
        if at == 17 {
            breaking = sha;
        }
    }
    let observer = SignedObserver::new(&w);
    let found = w
        .attribute(
            &[member("b", "TASK-010")],
            &observer,
            SearchBudget::default(),
        )
        .await;
    assert_eq!(found.regressions["b"].landing_commit, breaking);
    // The base, then ceil(log2(30)) = 5 bisection steps.
    assert!(observer.seen().len() <= 6, "{:?}", observer.seen());
}

/// The same failure read at another commit may differ in digits (timings,
/// counts), a clipped tail or a scratch path: still the tip's. Another
/// exit code or another message is not.
#[test]
fn a_failure_is_the_tips_by_its_signature_past_what_varies() {
    let tip = failure_signature(Some(1), "FAILED 3 of 20 in 2.10s at path=/tmp/a\n", "");
    let later = failure_signature(Some(1), "FAILED 4 of 21 in 9.87s at path=/x/b\n", "");
    assert!(Verdict::Failed(later).fails_as(&tip));
    let long = format!("Error: {}", "x".repeat(300));
    let clipped = format!("{} [...] tail", &long[..200]);
    let long_sig = failure_signature(Some(1), &long, "");
    assert!(Verdict::Failed(failure_signature(Some(1), &clipped, "")).fails_as(&long_sig));
    let other = failure_signature(Some(1), "Error: unrecognized subcommand\n", "");
    assert!(!Verdict::Failed(other).fails_as(&tip));
    let crashed = failure_signature(Some(101), "FAILED 3 of 20 in 2.10s\n", "");
    assert!(!Verdict::Failed(crashed).fails_as(&tip));
    assert!(Verdict::Held(false).fails_as(&tip));
    assert!(!Verdict::Held(true).fails_as(&tip));
}
