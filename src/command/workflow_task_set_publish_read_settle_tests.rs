//! Issue 338: a run's check-source read settles a publish a crash left under
//! the exclusive lock before it reads; one no read can settle pauses it; a
//! repin never overwrites the pins a settlement rolled forward; a chain
//! lock's recovery that cannot settle a journal is the same pause; and no
//! reader of the set falls back silently.

use std::path::{Path, PathBuf};

use archon_workflow::check_source_pins::{CheckSourcePins, PinStore, Repin, load_for_run};
use archon_workflow::check_source_resolve::Roots;
use archon_workflow::task_set_contract::{
    ACCEPTANCE_CONTRACT_FILE, AcceptanceContract, AcceptancePin, content_digest,
};
use archon_workflow::task_set_publish_lock::PublishLockError;

use super::journal::{Journal, JournalEntry, JournalPaths, JournalState, sibling_transaction_path};

/// Test support: a publish of `targets` (each with its new bytes) killed
/// with its journal in `state` and none of its renames done. When `stuck`,
/// the recovery log beside the pin cannot be written, so no read can settle
/// it. Returns the operator's fix: the log made writable again.
pub(crate) fn crash_publish(
    pin: &Path,
    targets: &[(PathBuf, Vec<u8>)],
    state: &str,
    stuck: bool,
) -> impl FnOnce() + use<> {
    let paths = JournalPaths::for_pin(pin);
    let transaction = uuid::Uuid::new_v4().simple().to_string();
    let entries = targets
        .iter()
        .map(|(target, bytes)| {
            let staged = sibling_transaction_path(target, &transaction, "new");
            std::fs::write(&staged, bytes).unwrap();
            JournalEntry {
                target: target.clone(),
                staged,
                backup: None,
                written: content_digest(bytes),
            }
        })
        .collect();
    let mut journal = Journal::new(transaction, "test", entries);
    journal.state = match state {
        "committed" => JournalState::Committed,
        _ => JournalState::Prepared,
    };
    journal.store(&paths).unwrap();
    if stuck {
        let _ = std::fs::remove_file(&paths.log);
        std::fs::create_dir_all(&paths.log).unwrap();
    }
    move || {
        if stuck {
            std::fs::remove_dir(&paths.log).unwrap();
        }
    }
}

/// A frozen set at version "v1" and the bytes of a whole version "v2" of
/// it: contract, pin and check-source sidecar.
struct Set {
    _temp: tempfile::TempDir,
    project: PathBuf,
    tasks: PathBuf,
    run: PathBuf,
}

impl Set {
    fn new() -> Self {
        crate::command::workflow_task_set::register_publish_settle();
        let temp = tempfile::tempdir().unwrap();
        let tasks = temp.path().join("tasks/PRD-X");
        std::fs::create_dir_all(&tasks).unwrap();
        let plain = |path: &Path| path.canonicalize().map(archon_shell::paths::plain).unwrap();
        let (project, tasks) = (plain(temp.path()), plain(&tasks));
        let set = Self {
            run: project.join(".archon/workflows/run-1"),
            _temp: temp,
            project,
            tasks,
        };
        std::fs::create_dir_all(set.pin().parent().unwrap()).unwrap();
        let [contract, pin, _] = set.version("v1");
        std::fs::write(&contract.0, &contract.1).unwrap();
        std::fs::write(&pin.0, &pin.1).unwrap();
        set.store().write(&set.pins("v1")).unwrap();
        set
    }

    fn pin(&self) -> PathBuf {
        crate::command::workflow_task_set::acceptance_pin_path(&self.project, &self.tasks)
    }

    fn store(&self) -> PinStore {
        PinStore::frozen(&self.project, &self.tasks)
    }

    fn paths(&self) -> JournalPaths {
        JournalPaths::for_pin(&self.pin())
    }

    fn contract_bytes(version: &str) -> Vec<u8> {
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema_version": 1, "prd": {"path": "p", "digest": version}, "gap_policy": {},
            "acceptance": [{"id": "AC-1", "criterion": version,
                "check": {"kind": "command", "command": "true", "cwd": "repo_root"},
                "judgment": {"verdict": "accepted", "counterexample": "n", "reason": "r", "host_call_id": "h"}}]
        }))
        .unwrap()
    }

    fn pins(&self, version: &str) -> CheckSourcePins {
        let bytes = Self::contract_bytes(version);
        let contract: AcceptanceContract = serde_json::from_slice(&bytes).unwrap();
        let roots = Roots {
            repository: &self.project,
            project: &self.project,
        };
        let mut pins = archon_workflow::check_source_pins::pin_contract(
            &contract,
            &content_digest(&bytes),
            &roots,
            "test",
            &self.store().blobs,
        );
        pins.pinned_at = "2026-10-06T00:00:00Z".into();
        pins
    }

    /// The whole of `version`: (path, bytes) of its contract, pin, sidecar.
    fn version(&self, version: &str) -> [(PathBuf, Vec<u8>); 3] {
        let contract = Self::contract_bytes(version);
        let sidecar = PinStore::bytes(&self.pins(version));
        let pin = serde_json::to_vec_pretty(&serde_json::json!({
            "task_root": self.tasks, "acceptance_digest": content_digest(&contract),
            "freeze_event_id": version, "check_sources_digest": content_digest(&sidecar),
            "acceptance_gate": {"mode": "enforce", "finding_count": 0, "findings_digest": "f", "binary_commit": "b", "evaluated_at": "t"}
        }))
        .unwrap();
        [
            (self.tasks.join(ACCEPTANCE_CONTRACT_FILE), contract),
            (self.pin(), pin),
            (self.store().sidecar, sidecar),
        ]
    }

    fn load(&self) -> Result<Option<(PinStore, CheckSourcePins)>, PublishLockError> {
        let roots = Roots {
            repository: &self.project,
            project: &self.project,
        };
        load_for_run(&self.run, &self.project, &self.tasks, &roots)
    }

    fn freeze_event_id(&self) -> String {
        let pin: AcceptancePin =
            serde_json::from_slice(&std::fs::read(self.pin()).unwrap()).unwrap();
        pin.freeze_event_id
    }
}

#[test]
fn a_run_read_settles_a_publish_left_mid_commit_and_reads_the_whole_new_set() {
    let set = Set::new();
    let _ = crash_publish(&set.pin(), &set.version("v2"), "committed", false);
    let (store, pins) = set.load().unwrap().unwrap();
    assert!(!set.paths().journal.exists(), "the read left the journal");
    assert!(store.frozen, "the frozen sidecar binds the new contract");
    assert_eq!(pins, set.pins("v2"), "a half-applied set was read");
    assert_eq!(set.freeze_event_id(), "v2");
}

#[test]
fn a_run_read_over_a_publish_that_never_committed_reads_the_whole_old_set() {
    let set = Set::new();
    let _ = crash_publish(&set.pin(), &set.version("v2"), "prepared", false);
    let (_, pins) = set.load().unwrap().unwrap();
    assert!(!set.paths().journal.exists(), "the read left the journal");
    assert_eq!(pins, set.pins("v1"));
    assert_eq!(set.freeze_event_id(), "v1");
}

#[test]
fn a_run_read_over_a_journal_no_read_can_settle_pauses_and_heals_once_fixed() {
    let set = Set::new();
    let fix = crash_publish(&set.pin(), &set.version("v2"), "committed", true);
    for _ in 0..2 {
        let error = set.load().expect_err("a read over an unsettled journal");
        let Some(evidence) = error.unsettled() else {
            panic!("the read failed instead of pausing: {error}");
        };
        for needed in [
            set.paths().journal.display().to_string(),
            "state: committed".into(),
            set.paths().log.display().to_string(),
            "Operator remedy".into(),
            "archon workflow resume --live --yes".into(),
        ] {
            assert!(evidence.contains(&needed), "{needed} missing: {evidence}");
        }
        assert!(
            set.paths().journal.exists(),
            "the journal's decision was lost"
        );
        assert!(!archon_workflow::task_set_publish_lock::held_here(
            &set.paths().lock
        ));
    }
    fix();
    let (_, pins) = set
        .load()
        .expect("the read settles once the cause is gone")
        .unwrap();
    assert_eq!(pins, set.pins("v2"));
    assert!(!set.paths().journal.exists());
}

#[test]
fn two_run_reads_that_find_one_left_journal_settle_it_once_and_read_the_same_set() {
    let set = Set::new();
    let _ = crash_publish(&set.pin(), &set.version("v2"), "committed", false);
    // Both readers queue behind a writer, so both find the journal.
    let writer = super::lock::PublishLock::acquire(&set.paths()).unwrap();
    let barrier = std::sync::Barrier::new(3);
    let read = std::thread::scope(|scope| {
        let readers: Vec<_> = (0..2)
            .map(|_| {
                scope.spawn(|| {
                    barrier.wait();
                    set.load().map(|loaded| loaded.unwrap().1)
                })
            })
            .collect();
        barrier.wait();
        std::thread::sleep(std::time::Duration::from_millis(300));
        drop(writer);
        readers
            .into_iter()
            .map(|reader| reader.join().unwrap().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(read, [set.pins("v2"), set.pins("v2")]);
    let log = std::fs::read_to_string(set.paths().log).unwrap();
    assert_eq!(log.matches("rolled forward").count(), 1, "{log}");
}

#[test]
fn a_repin_after_a_settled_publish_never_overwrites_the_pins_it_rolled_forward() {
    let set = Set::new();
    let read = set.pins("v1");
    // A publish of the same contract with other pins (another repin's),
    // killed after its commit point.
    let mut other = read.clone();
    other.origin = "another repin".into();
    let [_, _, mut sidecar] = set.version("v1");
    sidecar.1 = PinStore::bytes(&other);
    let mut pin: serde_json::Value = serde_json::from_slice(&set.version("v1")[1].1).unwrap();
    pin["check_sources_digest"] = content_digest(&sidecar.1).into();
    let pin = (set.pin(), serde_json::to_vec_pretty(&pin).unwrap());
    let _ = crash_publish(&set.pin(), &[pin, sidecar], "committed", false);
    let mut mine = read.clone();
    mine.origin = "mine".into();
    let store = set.store();
    assert_eq!(
        store.write_over(&read, &mine).unwrap(),
        Repin::Stale(other.clone())
    );
    assert!(!set.paths().journal.exists(), "the repin left the journal");
    assert_eq!(
        store.verified_read().unwrap().unwrap(),
        other,
        "overwritten"
    );
    assert_eq!(store.write_over(&other, &mine).unwrap(), Repin::Written);
    assert_eq!(store.verified_read().unwrap().unwrap(), mine);
}

#[test]
fn a_chain_lock_whose_recovery_cannot_settle_a_journal_is_an_unsettled_publish() {
    let set = Set::new();
    let fix = crash_publish(&set.pin(), &set.version("v2"), "committed", true);
    let recovered = super::recover_interrupted_publish(&set.pin(), &set.tasks).unwrap_err();
    assert!(super::UnsettledPublish::is(&recovered), "{recovered:#}");
    let locked = super::ChainLock::acquire(&set.pin(), &set.tasks)
        .err()
        .expect("a chain lock over an unsettled journal");
    let Some(archon_workflow::WorkflowError::ControlPaused(evidence)) =
        super::pause_if_unsettled(&locked)
    else {
        panic!("the chain lock's recovery failure is not a pause: {locked:#}");
    };
    assert!(evidence.contains("state: committed"), "{evidence}");
    assert!(evidence.contains("Operator remedy"), "{evidence}");
    // A chain lock another holder keeps is no journal: not a pause.
    fix();
    let held = super::ChainLock::acquire(&set.pin(), &set.tasks).unwrap();
    let busy = super::ChainLock::acquire(&set.pin(), &set.tasks)
        .err()
        .unwrap();
    assert!(super::pause_if_unsettled(&busy).is_none(), "{busy:#}");
    drop(held);
    assert_eq!(set.freeze_event_id(), "v2");
}

#[test]
fn a_child_that_reads_over_an_unsettled_journal_ends_with_the_documented_exit() {
    use crate::command::workflow_host_command_operational::{
        EXIT_UNSETTLED_PUBLISH, unsettled_publish_evidence, unsettled_publish_of,
    };
    let set = Set::new();
    let _fix = crash_publish(&set.pin(), &set.version("v2"), "committed", true);
    let error = crate::command::workflow_task_set::ChainRead::of(&set.project, &set.tasks)
        .err()
        .unwrap()
        .context("linting the set");
    let line = unsettled_publish_of(&error).expect("an unsettled read ends as one");
    assert!(!line.contains('\n'), "{line}");
    let evidence = unsettled_publish_evidence(
        Some(EXIT_UNSETTLED_PUBLISH),
        format!("progress\n{line}\n").as_bytes(),
    )
    .expect("the parent reads the child's evidence");
    assert!(evidence.contains("linting the set"), "{evidence}");
    assert!(evidence.contains("state: committed"), "{evidence}");
    // Any other failure is the command's own.
    let other = anyhow::anyhow!("the task set is malformed");
    assert!(unsettled_publish_of(&other).is_none());
}

#[tokio::test]
async fn a_judgment_reuse_over_an_unsettled_journal_stops_instead_of_rejudging_everything() {
    use crate::command::workflow_task_set::reauthor::test_client::ScriptedAuthorJudge;
    let set = Set::new();
    let fix = crash_publish(&set.pin(), &set.version("v2"), "committed", true);
    let client = ScriptedAuthorJudge::new(|_, _| String::new(), |_, _| true);
    let contract: AcceptanceContract = serde_json::from_slice(&Set::contract_bytes("v3")).unwrap();
    let expected = ["AC-1".to_string()].into_iter().collect();
    let error = super::super::incremental::judge(
        &set.project,
        &set.tasks,
        &client,
        contract.clone(),
        &expected,
        None,
    )
    .await
    .expect_err("no judgment is made over an unsettled journal");
    assert!(super::UnsettledPublish::is(&error), "{error:#}");
    assert!(
        format!("{error:#}").contains("judgments cannot be read"),
        "{error:#}"
    );
    assert!(client.judged_ids.lock().unwrap().is_empty(), "rejudged");
    fix();
    super::recover_interrupted_publish(&set.pin(), &set.tasks).unwrap();
    assert_eq!(set.freeze_event_id(), "v2");
}
