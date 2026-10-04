//! Issue 271: a task-set publish is all-or-nothing across a kill at any step.
//!
//! Each crash test re-runs this test binary as a child that publishes a fixed
//! set and aborts (SIGKILL: no destructors or cleanup) at a named step.
//! Kernel/device cache loss is not simulated; sync ordering is checked separately.
//! The parent then runs a recovery entry point and
//! requires the complete old set or the complete new set, with no debris.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use super::*;

const CHILD_ROOT_ENV: &str = "ARCHON_TEST_PUBLISH_CHILD_ROOT";
const OLD: [Option<&[u8]>; 4] = [Some(b"old-a"), Some(b"old-b"), None, Some(b"old-pin")];
const NEW: [Option<&[u8]>; 4] = [
    Some(b"new-a"),
    Some(b"new-b"),
    Some(b"new-c"),
    Some(b"new-pin"),
];
/// Every crash point of a publish, in order, and whether the publish had
/// passed its commit point there.
const STEPS: [(&str, bool); 9] = [
    ("prepared", false),
    ("staged", false),
    ("applying", false),
    ("renamed-0", false),
    ("renamed-1", false),
    ("renamed-2", false),
    ("renamed-3", false),
    ("committed", true),
    ("cleaned", true),
];

/// A task set at `<project>/tasks/set` with two existing files, one the
/// publish creates, and its acceptance pin in the project's pin store.
struct Scenario {
    project: tempfile::TempDir,
}

impl Scenario {
    fn new() -> Self {
        let project = tempfile::tempdir().unwrap();
        let tasks = project.path().join("tasks/set");
        std::fs::create_dir_all(&tasks).unwrap();
        std::fs::write(tasks.join("TASK-EX-001.md"), "task").unwrap();
        std::fs::write(tasks.join("a.json"), b"old-a").unwrap();
        std::fs::write(tasks.join("b.json"), b"old-b").unwrap();
        let scenario = Self { project };
        let pin = scenario.pin();
        std::fs::create_dir_all(pin.parent().unwrap()).unwrap();
        std::fs::write(&pin, b"old-pin").unwrap();
        scenario
    }

    fn root(&self) -> &Path {
        self.project.path()
    }

    fn tasks(&self) -> PathBuf {
        self.root().join("tasks/set")
    }

    fn pin(&self) -> PathBuf {
        crate::command::workflow_task_set::acceptance_pin_path(self.root(), &self.tasks())
    }

    fn targets(&self) -> [PathBuf; 4] {
        let tasks = self.tasks();
        [
            tasks.join("a.json"),
            tasks.join("b.json"),
            tasks.join("c.json"),
            self.pin(),
        ]
    }

    fn new_files(&self) -> Vec<(PathBuf, Vec<u8>)> {
        self.targets()
            .into_iter()
            .zip(NEW)
            .map(|(path, bytes)| (path, bytes.unwrap().to_vec()))
            .collect()
    }

    fn state(&self) -> Vec<Option<Vec<u8>>> {
        self.targets()
            .iter()
            .map(|path| std::fs::read(path).ok())
            .collect()
    }

    fn is(&self, expected: [Option<&[u8]>; 4]) -> bool {
        self.state()
            == expected
                .iter()
                .map(|bytes| bytes.map(<[u8]>::to_vec))
                .collect::<Vec<_>>()
    }

    /// Transaction files and journals left beside the set.
    fn debris(&self) -> Vec<String> {
        let pin_dir = self.pin().parent().unwrap().to_path_buf();
        [self.tasks(), pin_dir.clone(), pin_dir.join("check-sources")]
            .iter()
            .filter_map(|dir| std::fs::read_dir(dir).ok())
            .flatten()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| {
                name.ends_with(".new") || name.ends_with(".old") || name.contains("publish-journal")
            })
            .collect()
    }

    /// Run the child publisher, aborting at `step`; true when it was killed.
    fn publish_killed_at(&self, step: &str) -> bool {
        let marker = self.root().join("child-started");
        let _ = std::fs::remove_file(&marker);
        let evidence = self.root().join("crash-step");
        let _ = std::fs::remove_file(&evidence);
        let status = Command::new(std::env::current_exe().unwrap())
            .args([
                &child_test_name(),
                "--exact",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD_ROOT_ENV, self.root())
            .env(CRASH_ENV, step)
            .env(super::journal::CRASH_EVIDENCE_ENV, &evidence)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(marker.exists(), "the child publisher never ran");
        killed_at_crash_point(status, step, &evidence)
    }

    fn recovery_log(&self) -> String {
        std::fs::read_to_string(recovery_log_path(&self.pin())).unwrap_or_default()
    }
}

/// True only when the child died at the crash point (SIGKILL on Unix). A
/// panic or failed assertion in the child exits normally with status 101 and
/// is reported as such, never mistaken for an interruption.
fn killed_at_crash_point(status: std::process::ExitStatus, step: &str, evidence: &Path) -> bool {
    #[cfg(unix)]
    let killed = {
        use std::os::unix::process::ExitStatusExt;
        status.signal() == Some(libc::SIGKILL)
    };
    #[cfg(not(unix))]
    let killed = status.code() == Some(super::journal::NAMED_CRASH_EXIT_CODE);
    let named = std::fs::read_to_string(evidence).ok().as_deref() == Some(step);
    let killed = killed && named;
    assert!(
        killed || status.success(),
        "child failed at {step} without being killed there: {status}"
    );
    killed
}

fn child_test_name() -> String {
    let module = module_path!();
    let module = module.split_once("::").map_or(module, |(_, rest)| rest);
    format!("{module}::publish_crash_child")
}

/// The child half of every crash test; a no-op in a normal test run.
#[test]
fn publish_crash_child() {
    let Some(root) = std::env::var_os(CHILD_ROOT_ENV) else {
        return;
    };
    let root = PathBuf::from(root);
    std::fs::write(root.join("child-started"), b"").unwrap();
    let tasks = root
        .join(std::env::var_os("ARCHON_TEST_RECOVER_TASKS").unwrap_or_else(|| "tasks/set".into()));
    let pin = crate::command::workflow_task_set::acceptance_pin_path(&root, &tasks);
    let files = [
        tasks.join("a.json"),
        tasks.join("b.json"),
        tasks.join("c.json"),
        pin.clone(),
    ]
    .into_iter()
    .zip(NEW)
    .map(|(path, bytes)| (path, bytes.unwrap().to_vec()))
    .collect::<Vec<_>>();
    if std::env::var_os("ARCHON_TEST_RECOVER_CHILD").is_some() {
        recover_interrupted_publish(&pin, &tasks).unwrap();
    } else {
        publish(&pin, &files).expect("child publish");
    }
}

fn publish(pin: &Path, files: &[(PathBuf, Vec<u8>)]) -> Result<()> {
    publish_files_atomically(pin, files[0].0.parent().unwrap(), files, "test")
}

fn begin(pin: &Path, files: &[(PathBuf, Vec<u8>)]) -> Result<PublishTransaction> {
    begin_publish(pin, files[0].0.parent().unwrap(), files, "test", &[])
}

fn acquire_chain_lock(pin: &Path, tasks: &Path) -> Result<ChainLock> {
    ChainLock::acquire(pin, tasks)
}

#[test]
fn a_kill_at_any_publish_step_recovers_to_one_whole_set() {
    let mut wrong = Vec::new();
    for (step, committed) in STEPS {
        let scenario = Scenario::new();
        let killed = scenario.publish_killed_at(step);
        let report = recover_interrupted_publish(&scenario.pin(), &scenario.tasks());
        let expected = if committed { NEW } else { OLD };
        if !killed || report.is_err() || !scenario.is(expected) || !scenario.debris().is_empty() {
            wrong.push(format!(
                "{step}: killed={killed} recovery={:?} state={:?} debris={:?}",
                report.map(|report| report.events),
                scenario
                    .state()
                    .iter()
                    .map(|bytes| bytes.as_deref().map(String::from_utf8_lossy))
                    .collect::<Vec<_>>(),
                scenario.debris()
            ));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn a_kill_before_the_commit_point_rolls_back_at_launch() {
    let scenario = Scenario::new();
    assert!(scenario.publish_killed_at("renamed-1"));
    crate::command::workflow_task_set::republish::refuse_unaccepted_launch(
        scenario.root(),
        &scenario.tasks(),
    )
    .unwrap();
    assert!(scenario.is(OLD), "{:?}", scenario.state());
    assert!(scenario.debris().is_empty(), "{:?}", scenario.debris());
    assert!(
        scenario.recovery_log().contains("rolled back"),
        "{}",
        scenario.recovery_log()
    );
}

#[test]
fn a_kill_after_the_commit_point_rolls_forward_at_resume() {
    use archon_workflow::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};
    let scenario = Scenario::new();
    // Recovery must actually move staged bytes, not just clean backups.
    round_two::fixture(&scenario, JournalState::Committed, false);
    assert!(!scenario.is(NEW));
    let store = archon_workflow::WorkflowStore::project(scenario.root());
    let universe = WorkflowV2TaskUniverse {
        schema_version: "workflow-v2-task-universe-v1".into(),
        source_roots: vec![scenario.tasks().display().to_string()],
        tasks: vec![WorkflowV2TaskUniverseTask {
            canonical_task_id: "TASK-EX-001".into(),
            source_path: scenario
                .tasks()
                .join("TASK-EX-001.md")
                .display()
                .to_string(),
            ..WorkflowV2TaskUniverseTask::default()
        }],
    };
    crate::command::workflow_live::recover_bound_task_set(&store, Some(&universe)).unwrap();
    assert!(scenario.is(NEW), "{:?}", scenario.state());
    assert!(scenario.debris().is_empty(), "{:?}", scenario.debris());
    assert!(
        scenario.recovery_log().contains("rolled forward"),
        "{}",
        scenario.recovery_log()
    );
}

#[test]
fn a_kill_mid_publish_is_recovered_when_the_next_publisher_takes_the_lock() {
    let scenario = Scenario::new();
    assert!(scenario.publish_killed_at("renamed-2"));
    let _lock = acquire_chain_lock(&scenario.pin(), &scenario.tasks()).unwrap();
    assert!(scenario.is(OLD), "{:?}", scenario.state());
    assert!(scenario.debris().is_empty(), "{:?}", scenario.debris());
}

/// What the publisher before the journal left when killed between renames:
/// `a` replaced (backup kept), `b` backed up but not yet replaced, `c` and the
/// pin still only staged. That binary staged and fsynced every file before its
/// first rename, so the debris is rolled forward and logged, never refused.
#[test]
fn debris_from_an_older_binary_without_a_manifest_is_rolled_forward() {
    let scenario = Scenario::new();
    let [a, b, c, pin] = scenario.targets();
    let txn = "0123456789abcdef0123456789abcdef";
    let sibling = |path: &Path, role: &str| {
        let name = path.file_name().unwrap().to_string_lossy();
        path.with_file_name(format!(".{name}.{txn}.{role}"))
    };
    std::fs::rename(&a, sibling(&a, "old")).unwrap();
    std::fs::write(&a, b"new-a").unwrap();
    std::fs::hard_link(&b, sibling(&b, "old")).unwrap();
    std::fs::write(sibling(&b, "new"), b"new-b").unwrap();
    std::fs::write(sibling(&c, "new"), b"new-c").unwrap();
    std::fs::write(sibling(&pin, "new"), b"new-pin").unwrap();
    let report = recover_interrupted_publish(&scenario.pin(), &scenario.tasks()).unwrap();
    let mut expected = NEW;
    expected[3] = None;
    assert!(scenario.is(expected), "{:?}", scenario.state());
    assert!(scenario.debris().is_empty(), "{:?}", scenario.debris());
    assert!(
        report
            .events
            .iter()
            .any(|event| event.source == "legacy" && event.transaction == txn),
        "{:?}",
        report.events
    );
    assert!(
        scenario.recovery_log().contains(txn),
        "{}",
        scenario.recovery_log()
    );
}

/// Staging an older binary left with no backup is discarded; another
/// task set's transaction in the shared pin store is never touched.
#[test]
fn staging_debris_from_an_older_binary_without_a_backup_is_discarded() {
    let scenario = Scenario::new();
    let [a, ..] = scenario.targets();
    let stale = a.with_file_name(".a.json.fedcba9876543210fedcba9876543210.new");
    std::fs::write(&stale, b"new-a").unwrap();
    let pin_dir = scenario.pin().parent().unwrap().to_path_buf();
    let foreign = pin_dir.join(".other.json.fedcba9876543210fedcba9876543210.new");
    std::fs::write(&foreign, b"theirs").unwrap();
    recover_interrupted_publish(&scenario.pin(), &scenario.tasks()).unwrap();
    assert!(scenario.is(OLD), "{:?}", scenario.state());
    assert!(!stale.exists());
    assert!(foreign.exists(), "another set's staging must be left alone");
    assert!(
        scenario.recovery_log().contains("legacy"),
        "{}",
        scenario.recovery_log()
    );
}

#[test]
fn recovery_waits_for_a_live_publisher_and_never_undoes_it() {
    let scenario = Scenario::new();
    let transaction = begin(&scenario.pin(), &scenario.new_files()).unwrap();
    let (sender, receiver) = std::sync::mpsc::channel();
    let (pin, tasks) = (scenario.pin(), scenario.tasks());
    let recovering = std::thread::spawn(move || {
        let _ = sender.send(recover_interrupted_publish(&pin, &tasks).map(|r| r.events));
    });
    assert!(
        receiver.recv_timeout(Duration::from_millis(400)).is_err(),
        "recovery must wait while a live publisher holds the publish lock"
    );
    assert!(scenario.is(NEW), "{:?}", scenario.state());
    let _ = transaction.commit();
    let events = receiver
        .recv_timeout(Duration::from_secs(20))
        .expect("recovery finishes once the publisher commits")
        .unwrap();
    recovering.join().unwrap();
    assert!(events.is_empty(), "nothing to recover: {events:?}");
    assert!(scenario.is(NEW), "{:?}", scenario.state());
    assert!(scenario.debris().is_empty(), "{:?}", scenario.debris());
}

/// The journal is read back from disk, so a path in it that is not one of
/// its own transaction files beside its target is refused, never renamed.
#[test]
fn a_journal_naming_a_foreign_path_is_refused_and_touches_nothing() {
    let scenario = Scenario::new();
    let [a, ..] = scenario.targets();
    std::fs::write(&a, b"new-a").unwrap();
    let outside = tempfile::tempdir().unwrap();
    let victim = outside.path().join("victim.json");
    std::fs::write(&victim, b"victim").unwrap();
    let txn = "00112233445566778899aabbccddeeff";
    let journal = serde_json::json!({
        "schema_version": 1,
        "transaction": txn,
        "state": "applying",
        "remedy": "test",
        "entries": [{
            "target": a,
            "staged": a.with_file_name(format!(".a.json.{txn}.new")),
            "backup": victim,
            "written": archon_workflow::task_set_contract::content_digest(b"new-a"),
        }],
    });
    let journal_path = scenario.pin().with_extension("publish-journal");
    std::fs::write(&journal_path, serde_json::to_vec(&journal).unwrap()).unwrap();
    let error = recover_interrupted_publish(&scenario.pin(), &scenario.tasks())
        .expect_err("a foreign path is refused")
        .to_string();
    assert!(error.contains("not a transaction file"), "{error}");
    assert_eq!(std::fs::read(&victim).unwrap(), b"victim");
    assert_eq!(std::fs::read(&a).unwrap(), b"new-a");
    assert!(journal_path.exists(), "the journal is kept for inspection");
}

#[test]
fn a_kill_mid_publish_is_recovered_before_decomposition_reads_the_chain() {
    let scenario = Scenario::new();
    assert!(scenario.publish_killed_at("renamed-0"));
    crate::command::workflow_decompose_frozen_chain::frozen_chain_snapshot(
        scenario.root(),
        &scenario.root().join("prd.md"),
        &scenario.tasks(),
    )
    .unwrap();
    assert!(scenario.is(OLD), "{:?}", scenario.state());
    assert!(scenario.debris().is_empty(), "{:?}", scenario.debris());
}

#[cfg(test)]
#[path = "workflow_task_set_publish_round_three_tests.rs"]
mod round_three;
#[path = "workflow_task_set_publish_round_two_tests.rs"]
mod round_two;

#[path = "workflow_task_set_publish_round_four_tests.rs"]
mod round_four;
