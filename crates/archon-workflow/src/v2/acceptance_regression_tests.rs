//! A frozen acceptance check that held at its owner's landing and fails at
//! the tip is attributed to the first run landing it fails at, by
//! bisection over the run's landings, bounded and cached.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use super::{
    AcceptanceRegressionV1, CheckObserver, FailingCheck, MAX_OBSERVATIONS, attribute_regressions,
};
use crate::v2::{WorkflowV2DispatchedItem, WorkflowV2ResultStore};
use crate::write_coordinator::worktree_isolation::run_git;
use crate::{
    WorkflowV2CallRecord, WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2HostOptions,
    WorkflowV2Result,
};

const RUN: &str = "wf-test";

fn git(root: &Path, args: &[&str]) -> String {
    String::from_utf8(run_git(args, root).expect("git").stdout)
        .unwrap()
        .trim()
        .to_string()
}

/// Commit `file = content` as the run's landing of `stage`, or as a plain
/// commit when `stage` is `None`; returns the sha.
fn land(repo: &Path, file: &str, content: &str, stage: Option<&str>) -> String {
    std::fs::write(repo.join(file), content).unwrap();
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
        None => git(repo, &["commit", "-qm", "base"]),
    };
    git(repo, &["rev-parse", "HEAD"])
}

fn record_stage(store: &WorkflowV2ResultStore, stage: &str, tasks: &[&str]) {
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

/// Checks pass by reading `<id>.flag` at the commit: `ok` passes.
struct FlagObserver {
    repo: PathBuf,
    seen: Mutex<Vec<(String, Vec<String>)>>,
}

#[async_trait::async_trait]
impl CheckObserver for FlagObserver {
    async fn observe(&self, commit: &str, ids: &[String]) -> Option<BTreeMap<String, bool>> {
        self.seen
            .lock()
            .unwrap()
            .push((commit.to_string(), ids.to_vec()));
        Some(
            ids.iter()
                .map(|id| {
                    let shown = run_git(&["show", &format!("{commit}:{id}.flag")], &self.repo)
                        .map(|out| String::from_utf8_lossy(&out.stdout).trim() == "ok")
                        .unwrap_or(false);
                    (id.clone(), shown)
                })
                .collect(),
        )
    }
}

struct World {
    _dir: tempfile::TempDir,
    repo: PathBuf,
    store: WorkflowV2ResultStore,
    breaking: String,
    tip: String,
}

/// The owner's landing makes both checks hold; a later landing of another
/// task breaks `broken`; a third task lands after; `never` never held.
fn world() -> World {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["config", "user.name", "t"]);
    git(&repo, &["config", "user.email", "t@example.invalid"]);
    let base = land(&repo, "broken.flag", "missing", None);
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
    let step = |file: &str, content: &str, stage: &str, task: &str| {
        record_stage(&store, stage, &[task]);
        land(&repo, file, content, Some(stage))
    };
    step("broken.flag", "ok", "implement-owner-1", "TASK-OWNER");
    step("other.txt", "1", "implement-other-2", "TASK-OTHER");
    step("other.txt", "2", "implement-other-3", "TASK-OTHER");
    let breaking = step("broken.flag", "bad", "remediate-culprit-4", "TASK-CULPRIT");
    step("later.txt", "1", "implement-later-5", "TASK-LATER");
    let tip = step("later.txt", "2", "implement-later-6", "TASK-LATER");
    World {
        _dir: dir,
        repo,
        store,
        breaking,
        tip,
    }
}

fn failing(ids: &[&str]) -> Vec<FailingCheck> {
    ids.iter()
        .map(|id| FailingCheck {
            id: id.to_string(),
            owners: vec!["TASK-OWNER".into()],
        })
        .collect()
}

#[tokio::test]
async fn a_check_that_held_at_its_owners_landing_is_attributed_to_the_landing_that_broke_it() {
    let w = world();
    let observer = FlagObserver {
        repo: w.repo.clone(),
        seen: Mutex::new(Vec::new()),
    };
    let found = attribute_regressions(
        &w.store,
        &w.repo,
        &w.tip,
        &failing(&["broken", "never"]),
        &observer,
    )
    .await;
    assert_eq!(
        found.get("broken"),
        Some(&AcceptanceRegressionV1 {
            // The last point it still held: the landing just before.
            held_at: git(&w.repo, &["rev-parse", &format!("{}^", w.breaking)]),
            landing_commit: w.breaking.clone(),
            landing_stage: "remediate-culprit-4".into(),
            tasks: vec!["TASK-CULPRIT".into()],
        }),
        "{found:#?}"
    );
    // Never held at its owner's landing: its owner's, not attributed.
    assert!(!found.contains_key("never"), "{found:#?}");
    let observed = observer.seen.lock().unwrap().len();
    assert!(observed <= MAX_OBSERVATIONS, "{observed}");
    // Cached: a second round observes nothing.
    let again = FlagObserver {
        repo: w.repo.clone(),
        seen: Mutex::new(Vec::new()),
    };
    let second = attribute_regressions(
        &w.store,
        &w.repo,
        &w.tip,
        &failing(&["broken", "never"]),
        &again,
    )
    .await;
    assert_eq!(second, found);
    assert!(again.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn an_observation_that_fails_attributes_nothing() {
    struct Down;
    #[async_trait::async_trait]
    impl CheckObserver for Down {
        async fn observe(&self, _: &str, _: &[String]) -> Option<BTreeMap<String, bool>> {
            None
        }
    }
    let w = world();
    let found =
        attribute_regressions(&w.store, &w.repo, &w.tip, &failing(&["broken"]), &Down).await;
    assert!(found.is_empty(), "{found:#?}");
}
