//! Issue 336: a waiver and a repin settle an interrupted publish and write
//! under the publish lock; a journal no read can settle pauses the run.

use std::path::{Path, PathBuf};
use std::sync::mpsc::channel;
use std::time::Duration;

use archon_workflow::WorkflowError;
use archon_workflow::task_set_contract::{AcceptancePin, content_digest};

use super::journal::{Journal, JournalEntry, JournalPaths, JournalState, sibling_transaction_path};

const BLOCKED: Duration = Duration::from_millis(300);
const FINISHES: Duration = Duration::from_secs(20);

/// A frozen task set whose pin names `freeze_event_id` "old".
struct Set {
    _temp: tempfile::TempDir,
    project: PathBuf,
    tasks: PathBuf,
}

impl Set {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let tasks = temp.path().join("tasks/PRD-X");
        std::fs::create_dir_all(&tasks).unwrap();
        let plain = |path: &Path| path.canonicalize().map(archon_shell::paths::plain).unwrap();
        let (project, tasks) = (plain(temp.path()), plain(&tasks));
        let set = Self {
            _temp: temp,
            project,
            tasks,
        };
        std::fs::create_dir_all(set.pin().parent().unwrap()).unwrap();
        std::fs::write(set.pin(), set.pin_bytes("old")).unwrap();
        set
    }

    fn pin(&self) -> PathBuf {
        crate::command::workflow_task_set::acceptance_pin_path(&self.project, &self.tasks)
    }

    fn pin_bytes(&self, freeze_event_id: &str) -> Vec<u8> {
        serde_json::to_vec_pretty(&serde_json::json!({
            "task_root": self.tasks,
            "acceptance_digest": "d",
            "freeze_event_id": freeze_event_id,
            "acceptance_gate": {"mode": "enforce", "finding_count": 0, "findings_digest": "f", "binary_commit": "b", "evaluated_at": "t"}
        }))
        .unwrap()
    }

    fn read_pin(&self) -> AcceptancePin {
        serde_json::from_slice(&std::fs::read(self.pin()).unwrap()).unwrap()
    }

    fn paths(&self) -> JournalPaths {
        JournalPaths::for_pin(&self.pin())
    }

    /// A publish of the pin "new" killed after its commit point and before
    /// its rename: the journal is committed, the pin still the old one.
    fn crash_after_commit(&self) {
        let transaction = uuid::Uuid::new_v4().simple().to_string();
        let staged = sibling_transaction_path(&self.pin(), &transaction, "new");
        let bytes = self.pin_bytes("new");
        std::fs::write(&staged, &bytes).unwrap();
        let mut journal = Journal::new(
            transaction,
            "test",
            vec![JournalEntry {
                target: self.pin(),
                staged,
                backup: None,
                written: content_digest(&bytes),
            }],
        );
        journal.state = JournalState::Committed;
        journal.store(&self.paths()).unwrap();
    }

    fn waive(&self) -> anyhow::Result<PathBuf> {
        let waivers = crate::command::topology_lint::waivers_from_flags(
            &["G-X-001".into()],
            Some("lands next sprint"),
        )
        .unwrap();
        crate::command::topology_lint::record_waivers(&self.project, &self.tasks, &waivers)
    }
}

fn waived(pin: &AcceptancePin) -> bool {
    pin.fidelity_waivers
        .iter()
        .any(|waiver| waiver.obligation_id == "G-X-001")
}

#[test]
fn the_host_and_the_repins_name_the_same_journal() {
    let set = Set::new();
    let names = archon_workflow::task_set_publish_lock::journal_paths(&set.pin());
    assert_eq!(names, [set.paths().journal, set.paths().journal_temp()]);
    assert_eq!(
        archon_workflow::task_set_publish_lock::lock_path(&set.pin()),
        set.paths().lock
    );
}

#[test]
fn a_waiver_waits_for_a_publish_in_progress_and_keeps_both() {
    let set = Set::new();
    let publish = super::begin_publish(
        &set.pin(),
        &set.tasks,
        &[(set.pin(), set.pin_bytes("new"))],
        "test",
        &[],
    )
    .unwrap();
    let (tx, rx) = channel();
    let waiver = std::thread::scope(|scope| {
        let waiver = scope.spawn(|| {
            let result = set.waive();
            tx.send(()).unwrap();
            result
        });
        assert!(
            rx.recv_timeout(BLOCKED).is_err(),
            "the waiver rewrote the pin while a publish of it was open"
        );
        let warnings = publish.commit().unwrap();
        assert!(
            warnings.is_empty(),
            "unexpected commit warnings: {warnings:?}"
        );
        rx.recv_timeout(FINISHES)
            .expect("the waiver ran once the publish ended");
        waiver.join().unwrap()
    });
    waiver.unwrap();
    let pin = set.read_pin();
    assert_eq!(pin.freeze_event_id, "new", "the publish was overwritten");
    assert!(waived(&pin), "the waiver was lost");
}

#[test]
fn a_waiver_after_a_crash_settles_the_publish_first() {
    let set = Set::new();
    set.crash_after_commit();
    set.waive().unwrap();
    assert!(!set.paths().journal.exists(), "the waiver left the journal");
    // The next acquisition must not roll a stale publish over the waiver.
    super::recover_interrupted_publish(&set.pin(), &set.tasks).unwrap();
    let pin = set.read_pin();
    assert_eq!(pin.freeze_event_id, "new", "the committed publish was lost");
    assert!(waived(&pin), "the waiver was lost to the roll-forward");
}

#[test]
fn a_repin_after_a_crash_settles_the_publish_first() {
    use archon_workflow::check_source_pins::{CheckSourcePins, PinStore};
    crate::command::workflow_task_set::register_publish_settle();
    let set = Set::new();
    set.crash_after_commit();
    let pins = CheckSourcePins {
        schema_version: 1,
        acceptance_digest: "d".into(),
        origin: "test".into(),
        pinned_at: "2026-10-05T00:00:00Z".into(),
        checks: Default::default(),
        repins: Vec::new(),
    };
    PinStore::frozen(&set.project, &set.tasks)
        .write(&pins)
        .unwrap();
    assert!(!set.paths().journal.exists(), "the repin left the journal");
    super::recover_interrupted_publish(&set.pin(), &set.tasks).unwrap();
    let pin = set.read_pin();
    assert_eq!(pin.freeze_event_id, "new", "the committed publish was lost");
    assert!(
        pin.check_sources_digest.is_some(),
        "the repin was lost to the roll-forward"
    );
}

#[test]
fn a_journal_no_read_can_settle_pauses_the_run_and_heals_once_fixed() {
    let set = Set::new();
    set.crash_after_commit();
    // The recovery log cannot be written, so the committed journal's cleanup
    // fails on every read.
    let log = set.paths().log;
    std::fs::create_dir_all(&log).unwrap();
    for _ in 0..2 {
        let error =
            crate::command::workflow_task_set::ChainRead::workflow_at(&set.pin(), &set.tasks)
                .err()
                .expect("a read over an unsettled journal");
        let WorkflowError::ControlPaused(evidence) = error else {
            panic!("the read failed the run instead of pausing it: {error}");
        };
        for needed in [
            set.paths().journal.display().to_string(),
            "state: committed".into(),
            log.display().to_string(),
            "Operator remedy".into(),
            "archon workflow resume --live --yes".into(),
        ] {
            assert!(evidence.contains(&needed), "{needed} missing: {evidence}");
        }
        assert!(
            set.paths().journal.exists(),
            "the journal decision was lost"
        );
        let lock = set.paths().lock;
        assert!(!archon_workflow::task_set_publish_lock::held_here(&lock));
    }
    let anyhow = crate::command::workflow_task_set::ChainRead::begin(&set.pin(), &set.tasks)
        .err()
        .unwrap();
    assert!(super::lock::UnsettledPublish::is(&anyhow));
    // The operator's fix: the next read settles it.
    std::fs::remove_dir(&log).unwrap();
    let read = crate::command::workflow_task_set::ChainRead::workflow_at(&set.pin(), &set.tasks)
        .expect("the read settled the journal once its cause was gone");
    drop(read);
    assert!(!set.paths().journal.exists());
    assert_eq!(set.read_pin().freeze_event_id, "new");
}

/// Test support: the next publish this thread commits leaves its committed
/// journal behind, as if its removal had failed, and the recovery log beside
/// the pin cannot be written, so no read can settle it. Returns the
/// operator's fix: the log made writable again.
pub(crate) fn stick_next_commit(pin: &Path) -> impl FnOnce() + use<> {
    use std::cell::{Cell, RefCell};
    let paths = JournalPaths::for_pin(pin);
    let (hook, saved, done) = (paths.clone(), RefCell::new(None), Cell::new(false));
    super::journal::test_hooks::STEP.with(|step_hook| {
        *step_hook.borrow_mut() = Some(Box::new(move |step: &str| match step {
            _ if done.get() => {}
            "committed" => *saved.borrow_mut() = std::fs::read(&hook.journal).ok(),
            "cleaned" => {
                if let Some(bytes) = saved.borrow_mut().take() {
                    std::fs::write(&hook.journal, bytes).unwrap();
                    let _ = std::fs::remove_file(&hook.log);
                    std::fs::create_dir_all(&hook.log).unwrap();
                    done.set(true);
                }
            }
            _ => {}
        }))
    });
    move || std::fs::remove_dir(&paths.log).unwrap()
}

#[test]
fn a_read_whose_settlements_run_out_pauses_the_run_with_evidence() {
    let set = Set::new();
    set.crash_after_commit();
    let mut calls = 0;
    // Every settlement "succeeds" and leaves the journal.
    let error = super::lock::PublishLock::acquire_shared_settled(&set.pin(), || {
        calls += 1;
        Ok(())
    })
    .err()
    .expect("a journal left after every settlement is not read");
    assert_eq!(calls, 3);
    assert!(super::lock::UnsettledPublish::is(&error), "{error:#}");
    let WorkflowError::ControlPaused(evidence) = super::lock::stage_error(error) else {
        panic!("exhausted settlements failed the run instead of pausing it");
    };
    for needed in [
        set.paths().journal.display().to_string(),
        "state: committed".into(),
        "3 attempts".into(),
        "Operator remedy".into(),
    ] {
        assert!(evidence.contains(&needed), "{needed} missing: {evidence}");
    }
}
