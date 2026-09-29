//! A frozen acceptance check that held at any point of the run and fails at
//! the tip is attributed to the first run landing it fails at, by bisection
//! over the run's landings, bounded and cached; every other failing check
//! comes back with what the search established (Batch J).
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use super::{
    AcceptanceRegressionV1, Attribution, CheckObserver, FailingCheck, MAX_OBSERVATIONS,
    SearchBudget, Verdict, attribute_regressions, failure_signature,
};
use crate::v2::{WorkflowV2DispatchedItem, WorkflowV2ResultStore};
use crate::write_coordinator::worktree_isolation::run_git;
use crate::{
    WorkflowV2CallRecord, WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2HostOptions,
    WorkflowV2Result,
};

pub(crate) const RUN: &str = "wf-test";

pub(crate) fn git(root: &Path, args: &[&str]) -> String {
    String::from_utf8(run_git(args, root).expect("git").stdout)
        .unwrap()
        .trim()
        .to_string()
}

/// Commit `file = content` as the run's landing of `stage`, or as a plain
/// commit when `stage` is `None`; returns the sha.
pub(crate) fn land(repo: &Path, file: &str, content: &str, stage: Option<&str>) -> String {
    let path = repo.join(file);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
    git(repo, &["add", "."]);
    match stage {
        Some(stage) => git(
            repo,
            &[
                "commit",
                "-qm",
                &format!("archon: wave 0 outputs (run {RUN}, stage {stage})"),
                "--author",
                "archon-workflow <host@example.invalid>",
            ],
        ),
        None => git(repo, &["commit", "-qm", "plain"]),
    };
    git(repo, &["rev-parse", "HEAD"])
}

pub(crate) fn record_stage(store: &WorkflowV2ResultStore, stage: &str, tasks: &[&str]) {
    let call = WorkflowV2HostCall {
        id: stage.into(),
        method: WorkflowV2HostMethod::Fanout,
        write_mode: None,
        options: WorkflowV2HostOptions::default(),
    };
    let record = WorkflowV2CallRecord::new(
        RUN,
        call,
        1,
        "h".into(),
        WorkflowV2Result::accepted("landed"),
        vec![],
    )
    .with_dispatched_items(vec![WorkflowV2DispatchedItem {
        item_id: format!("{stage}-0"),
        canonical_task_ids: tasks.iter().map(|t| t.to_string()).collect(),
    }]);
    store.save_call_record(&record).unwrap();
}

/// Checks pass by reading `flags/<id>` at the commit: `ok` passes.
pub(crate) struct FlagObserver {
    pub(crate) repo: PathBuf,
    pub(crate) seen: Mutex<Vec<(String, Vec<String>)>>,
}

impl FlagObserver {
    pub(crate) fn new(repo: &Path) -> Self {
        Self {
            repo: repo.to_path_buf(),
            seen: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn observations(&self) -> usize {
        self.seen.lock().unwrap().len()
    }
}

#[async_trait::async_trait]
impl CheckObserver for FlagObserver {
    async fn observe(&self, commit: &str, ids: &[String]) -> Option<BTreeMap<String, Verdict>> {
        self.seen
            .lock()
            .unwrap()
            .push((commit.to_string(), ids.to_vec()));
        Some(
            ids.iter()
                .map(|id| {
                    let shown = run_git(&["show", &format!("{commit}:flags/{id}")], &self.repo)
                        .map(|out| String::from_utf8_lossy(&out.stdout).trim() == "ok")
                        .unwrap_or(false);
                    (id.clone(), Verdict::Held(shown))
                })
                .collect(),
        )
    }
}

pub(crate) struct World {
    _dir: tempfile::TempDir,
    pub(crate) repo: PathBuf,
    pub(crate) store: WorkflowV2ResultStore,
}

impl World {
    /// A repository whose base commit holds `base` (`(flag, content)`s).
    pub(crate) fn new(base: &[(&str, &str)]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q"]);
        git(&repo, &["config", "user.name", "t"]);
        git(&repo, &["config", "user.email", "t@example.invalid"]);
        for (flag, content) in base {
            std::fs::create_dir_all(repo.join("flags")).unwrap();
            std::fs::write(repo.join("flags").join(flag), content).unwrap();
        }
        let base = land(&repo, "README", "base", None);
        let store = WorkflowV2ResultStore::new(dir.path().join("run/v2"));
        std::fs::create_dir_all(dir.path().join("run")).unwrap();
        std::fs::write(
            dir.path().join("run/events.jsonl"),
            format!(
                "{}\n",
                serde_json::json!({"seq": 1, "kind": "started",
                    "detail": {"event": "repository_bound", "head": base}})
            ),
        )
        .unwrap();
        Self {
            _dir: dir,
            repo,
            store,
        }
    }

    /// A run landing of `stage` by `task` writing `file = content`.
    pub(crate) fn step(&self, file: &str, content: &str, stage: &str, task: &str) -> String {
        record_stage(&self.store, stage, &[task]);
        land(&self.repo, file, content, Some(stage))
    }

    pub(crate) fn tip(&self) -> String {
        git(&self.repo, &["rev-parse", "HEAD"])
    }

    pub(crate) async fn attribute(
        &self,
        failing: &[FailingCheck],
        observer: &dyn CheckObserver,
        budget: SearchBudget,
    ) -> Attribution {
        attribute_regressions(
            &self.store,
            &self.repo,
            &self.tip(),
            failing,
            observer,
            budget,
        )
        .await
    }
}

pub(crate) fn check(id: &str, owner: &str) -> FailingCheck {
    FailingCheck {
        id: id.into(),
        owners: vec![owner.into()],
        signature: failure_signature(Some(1), &format!("Error: {id} failed"), ""),
        fingerprint: "f1".into(),
    }
}

/// The owner's landing makes `broken` hold; a later landing of another task
/// breaks it; `never` never holds anywhere.
#[tokio::test]
async fn a_check_that_held_at_its_owners_landing_is_attributed_to_the_landing_that_broke_it() {
    let w = World::new(&[("broken", "missing")]);
    w.step("flags/broken", "ok", "implement-owner-1", "TASK-OWNER");
    w.step("other.txt", "1", "implement-other-2", "TASK-OTHER");
    w.step("other.txt", "2", "implement-other-3", "TASK-OTHER");
    let breaking = w.step("flags/broken", "bad", "remediate-culprit-4", "TASK-CULPRIT");
    w.step("later.txt", "1", "implement-later-5", "TASK-LATER");
    w.step("later.txt", "2", "implement-later-6", "TASK-LATER");
    let failing = [check("broken", "TASK-OWNER"), check("never", "TASK-OWNER")];
    let observer = FlagObserver::new(&w.repo);
    let found = w
        .attribute(&failing, &observer, SearchBudget::default())
        .await;
    assert_eq!(
        found.regressions.get("broken"),
        Some(&AcceptanceRegressionV1 {
            // The last point it still held: the landing just before.
            held_at: git(&w.repo, &["rev-parse", &format!("{breaking}^")]),
            landing_commit: breaking.clone(),
            landing_stage: "remediate-culprit-4".into(),
            tasks: vec!["TASK-CULPRIT".into()],
            changed_files: vec!["flags/broken".into()],
            probed_as: None,
        }),
        "{found:#?}"
    );
    // Failing this way at the base and at its owner's landing: no landing
    // of the run is shown to have broken it (Batch J2).
    let never = &found.searches["never"];
    assert!(!never.never_held, "{never:?}");
    assert!(
        never
            .note
            .contains("already fails this way at the run base"),
        "{never:?}"
    );
    assert!(observer.observations() <= MAX_OBSERVATIONS);
    // Cached: a second round observes nothing and finds the same.
    let again = FlagObserver::new(&w.repo);
    let second = w.attribute(&failing, &again, SearchBudget::default()).await;
    assert_eq!(second, found);
    assert_eq!(again.observations(), 0);
}

/// The live wf-0ddadd81 shape: the check held at the run base, a landing of
/// another task broke it, and the owner landed AFTER the break, so its own
/// latest landing already fails. Before Batch J the search gave up there.
#[tokio::test]
async fn a_break_before_the_owners_latest_landing_is_found_from_the_run_base() {
    let w = World::new(&[("ac", "ok")]);
    w.step("a.txt", "1", "implement-a-1", "TASK-A");
    let breaking = w.step("flags/ac", "bad", "review-remediate-x-2", "TASK-X");
    w.step("owner.txt", "1", "review-remediate-owner-3", "TASK-OWNER");
    w.step("b.txt", "1", "implement-b-4", "TASK-B");
    let observer = FlagObserver::new(&w.repo);
    let found = w
        .attribute(
            &[check("ac", "TASK-OWNER")],
            &observer,
            SearchBudget::default(),
        )
        .await;
    let regression = &found.regressions["ac"];
    assert_eq!(regression.landing_commit, breaking);
    assert_eq!(regression.tasks, ["TASK-X"]);
    assert_eq!(regression.changed_files, ["flags/ac"]);
}

/// Batch J (b): five failing checks, each broken by a different landing,
/// are all attributed; before, only the first three by id were searched.
#[tokio::test]
async fn five_failing_checks_are_all_attributed() {
    let ids = ["c1", "c2", "c3", "c4", "c5"];
    let base: Vec<(&str, &str)> = ids.iter().map(|id| (*id, "ok")).collect();
    let w = World::new(&base);
    let mut breaks = BTreeMap::new();
    for (at, id) in ids.iter().enumerate() {
        w.step(
            &format!("pad{at}.txt"),
            "1",
            &format!("implement-pad-{at}"),
            "TASK-PAD",
        );
        let stage = format!("review-remediate-{id}-{at}");
        let task = format!("TASK-BREAKS-{id}");
        breaks.insert(
            *id,
            (w.step(&format!("flags/{id}"), "bad", &stage, &task), task),
        );
    }
    let failing: Vec<FailingCheck> = ids.iter().map(|id| check(id, "TASK-OWNER")).collect();
    let observer = FlagObserver::new(&w.repo);
    let found = w
        .attribute(&failing, &observer, SearchBudget::default())
        .await;
    assert!(found.searches.is_empty(), "{found:#?}");
    for id in ids {
        let regression = &found.regressions[id];
        assert_eq!(regression.landing_commit, breaks[id].0, "{id}");
        assert_eq!(regression.tasks, [breaks[id].1.clone()], "{id}");
    }
    // One observation serves every check probed at the same commit.
    assert!(
        observer.observations() < 5 * 4,
        "{}",
        observer.observations()
    );
}

/// Checks failing identically are searched once: a seek point observes
/// every member in one observation (Batch J2), a bisection only the lead;
/// the others are confirmed at the landing pinned.
#[tokio::test]
async fn checks_failing_identically_are_probed_once() {
    let w = World::new(&[("a", "ok"), ("b", "ok"), ("c", "ok")]);
    w.step("x.txt", "1", "implement-x-1", "TASK-X");
    let breaking = w.step("flags/a", "bad", "review-remediate-y-2", "TASK-Y");
    w.step("flags/b", "bad", "review-remediate-y-3", "TASK-Y");
    w.step("flags/c", "bad", "review-remediate-y-4", "TASK-Y");
    let same = failure_signature(Some(1), "warning: x\nError: unknown asset_class `u`\n", "");
    let failing: Vec<FailingCheck> = ["a", "b", "c"]
        .iter()
        .map(|id| FailingCheck {
            signature: same.clone(),
            ..check(id, "TASK-OWNER")
        })
        .collect();
    let observer = FlagObserver::new(&w.repo);
    let found = w
        .attribute(&failing, &observer, SearchBudget::default())
        .await;
    let seen = observer.seen.lock().unwrap().clone();
    let base = git(&w.repo, &["rev-list", "--max-parents=0", "HEAD"]);
    // The owner has no landing: the base is the only primary point, and
    // every member is observed there at once.
    assert_eq!(seen[0], (base, vec!["a".into(), "b".into(), "c".into()]));
    // Then the bisection asks the lead alone, until its landing is pinned.
    let first = (seen.iter().skip(1))
        .position(|(_, ids)| ids.iter().any(|id| id != "a"))
        .expect("the others are confirmed")
        + 1;
    assert!(
        seen[1..first].iter().all(|(_, ids)| ids == &["a"]),
        "{seen:?}"
    );
    let held_at = git(&w.repo, &["rev-parse", &format!("{breaking}^")]);
    assert!(
        seen[first].0 == held_at || seen[first].0 == breaking,
        "{seen:?}"
    );
    // b and c fail identically but broke at other landings: each is split
    // off and pinned on its own, never handed a's landing.
    assert_eq!(found.regressions["a"].landing_commit, breaking);
    assert_eq!(found.regressions["a"].probed_as, None);
    for id in ["b", "c"] {
        let regression = &found.regressions[id];
        assert_ne!(regression.landing_commit, breaking, "{id}");
        assert_eq!(regression.changed_files, [format!("flags/{id}")], "{id}");
        assert_eq!(regression.probed_as, None, "{id}");
    }
}

/// A member that does share the probe's break is confirmed and shares it.
#[tokio::test]
async fn a_member_confirmed_at_the_break_shares_it() {
    let w = World::new(&[("a", "ok"), ("b", "ok")]);
    w.step("x.txt", "1", "implement-x-1", "TASK-X");
    // One landing breaks both.
    std::fs::write(w.repo.join("flags/b"), "bad").unwrap();
    let breaking = w.step("flags/a", "bad", "review-remediate-y-2", "TASK-Y");
    w.step("z.txt", "1", "implement-z-3", "TASK-Z");
    let same = failure_signature(Some(1), "Error: same\n", "");
    let failing: Vec<FailingCheck> = ["a", "b"]
        .iter()
        .map(|id| FailingCheck {
            signature: same.clone(),
            ..check(id, "TASK-OWNER")
        })
        .collect();
    let observer = FlagObserver::new(&w.repo);
    let found = w
        .attribute(&failing, &observer, SearchBudget::default())
        .await;
    assert_eq!(found.regressions["b"].landing_commit, breaking);
    assert_eq!(found.regressions["b"].probed_as.as_deref(), Some("a"));
}

/// A break after the run's last landing, in a commit the run did not land,
/// is laid at no task; the note says so.
#[tokio::test]
async fn a_break_the_run_did_not_land_names_no_task() {
    let w = World::new(&[("a", "ok")]);
    w.step("x.txt", "1", "implement-x-1", "TASK-X");
    land(&w.repo, "flags/a", "bad", None);
    let observer = FlagObserver::new(&w.repo);
    let found = w
        .attribute(
            &[check("a", "TASK-OWNER")],
            &observer,
            SearchBudget::default(),
        )
        .await;
    assert!(found.regressions.is_empty(), "{found:#?}");
    assert!(
        found.searches["a"].note.contains("did not land"),
        "{found:#?}"
    );
}

#[tokio::test]
async fn an_observation_that_fails_attributes_nothing_and_drops_nothing() {
    struct Down;
    #[async_trait::async_trait]
    impl CheckObserver for Down {
        async fn observe(&self, _: &str, _: &[String]) -> Option<BTreeMap<String, Verdict>> {
            None
        }
    }
    let w = World::new(&[("a", "ok")]);
    w.step("flags/a", "bad", "review-remediate-y-1", "TASK-Y");
    let found = w
        .attribute(&[check("a", "TASK-OWNER")], &Down, SearchBudget::default())
        .await;
    assert!(found.regressions.is_empty(), "{found:#?}");
    let search = &found.searches["a"];
    assert!(
        !search.never_held && search.note.contains("could not be observed"),
        "{search:?}"
    );
}

/// A landing pinned but with no task the run's records name is never a
/// route to nobody: the note names the landing and what it changed.
#[tokio::test]
async fn a_pinned_landing_with_no_recorded_task_is_named_in_the_note() {
    let w = World::new(&[("a", "ok")]);
    w.step("x.txt", "1", "implement-x-1", "TASK-X");
    let breaking = land(&w.repo, "flags/a", "bad", Some("unrecorded-stage-2"));
    w.step("z.txt", "1", "implement-z-3", "TASK-Z");
    let observer = FlagObserver::new(&w.repo);
    let found = w
        .attribute(
            &[check("a", "TASK-OWNER")],
            &observer,
            SearchBudget::default(),
        )
        .await;
    assert!(found.regressions.is_empty(), "{found:#?}");
    let note = &found.searches["a"].note;
    assert!(
        note.contains(&breaking) && note.contains("flags/a") && note.contains("no task"),
        "{note}"
    );
}
