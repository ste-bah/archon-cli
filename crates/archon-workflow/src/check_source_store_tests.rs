//! Issue 338: a run's read of its check-source pins settles a publish a
//! crash left before it reads, pauses on one no read can settle, and never
//! writes; a repin is never written over pins it did not start from.

use std::sync::mpsc::channel;
use std::time::Duration;

use super::*;
use crate::check_source_pins::ORIGIN_FREEZE;
use crate::check_source_pins::tests_support::contract;
use crate::task_set_publish_lock::{journal_paths, lock_path};

const BLOCKED: Duration = Duration::from_millis(300);
const FINISHES: Duration = Duration::from_secs(20);

/// A frozen set: its contract, its pin naming its sidecar, the sidecar.
struct Frozen {
    _dir: tempfile::TempDir,
    repo: PathBuf,
    project: PathBuf,
    tasks: PathBuf,
    run: PathBuf,
    store: PinStore,
    pins: CheckSourcePins,
}

impl Frozen {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let (repo, project) = (dir.path().join("repo"), dir.path().join("project"));
        let tasks = project.join("tasks/set");
        std::fs::create_dir_all(repo.join("scripts")).unwrap();
        std::fs::create_dir_all(&tasks).unwrap();
        std::fs::write(repo.join("scripts/a.sh"), "exit 1\n").unwrap();
        let bytes = serde_json::to_vec_pretty(&contract(&[("AC-1", "bash scripts/a.sh")])).unwrap();
        std::fs::write(tasks.join(ACCEPTANCE_CONTRACT_FILE), &bytes).unwrap();
        let store = PinStore::frozen(&project, &tasks);
        let pin = store.pin.clone().unwrap();
        std::fs::create_dir_all(pin.parent().unwrap()).unwrap();
        let digest = content_digest(&bytes);
        let pin_json = serde_json::json!({
            "task_root": tasks.display().to_string(), "acceptance_digest": digest,
            "freeze_event_id": "acceptance-freeze-d",
            "acceptance_gate": {"mode": "enforce", "finding_count": 0, "findings_digest": "x",
                "binary_commit": "c", "evaluated_at": "2026-01-01T00:00:00Z"}
        });
        std::fs::write(&pin, serde_json::to_vec_pretty(&pin_json).unwrap()).unwrap();
        let roots = Roots {
            repository: &repo,
            project: &project,
        };
        let c: AcceptanceContract = serde_json::from_slice(&bytes).unwrap();
        let pins = crate::check_source_pins::pin_contract(
            &c,
            &digest,
            &roots,
            ORIGIN_FREEZE,
            &store.blobs,
        );
        store.write(&pins).unwrap();
        Self {
            run: project.join(".archon/workflows/run-1"),
            _dir: dir,
            repo,
            project,
            tasks,
            store,
            pins,
        }
    }

    fn pin(&self) -> PathBuf {
        self.store.pin.clone().unwrap()
    }

    fn load(&self) -> Result<Option<(PinStore, CheckSourcePins)>, PublishLockError> {
        let roots = Roots {
            repository: &self.repo,
            project: &self.project,
        };
        load_for_run(&self.run, &self.project, &self.tasks, &roots)
    }

    /// The sidecar left behind its pin, as a repin killed between its pin
    /// and its sidecar leaves it.
    fn behind(&self) -> Vec<u8> {
        let stale = b"{\"left\": \"behind\"}".to_vec();
        std::fs::write(&self.store.sidecar, &stale).unwrap();
        stale
    }
}

#[test]
fn a_publish_left_with_no_settlement_pauses_the_read_with_its_evidence() {
    // No host is linked into this crate's tests, so no settlement is
    // installed: a journal, or a journal temp alone, cannot be settled.
    for (index, state) in [(0, "committed"), (1, "a journal temp only")] {
        let set = Frozen::new();
        let left = journal_paths(&set.pin())[index].clone();
        std::fs::write(&left, br#"{"state": "committed"}"#).unwrap();
        let error = set.load().expect_err("a read over a left journal");
        let evidence = error.unsettled().expect("an unsettled publish pauses");
        for needed in [
            journal_paths(&set.pin())[0].display().to_string(),
            format!("state: {state}"),
            "no settlement".into(),
            "Operator remedy".into(),
            "archon workflow resume --live --yes".into(),
        ] {
            assert!(evidence.contains(&needed), "{needed} missing: {evidence}");
        }
        assert!(left.exists(), "the journal's decision was lost");
        assert!(!crate::task_set_publish_lock::held_here(&lock_path(
            &set.pin()
        )));
        // Once it is gone, the same read goes on.
        std::fs::remove_file(&left).unwrap();
        assert!(set.load().unwrap().unwrap().0.frozen);
    }
}

#[test]
fn a_sidecar_left_behind_its_pin_is_put_back_only_under_the_exclusive_lock() {
    let set = Frozen::new();
    let stale = set.behind();
    // Another run reads the set: the put-back must wait for it.
    let reader = PublishLockFile::hold_shared(&set.pin()).unwrap().unwrap();
    let (tx, rx) = channel();
    std::thread::scope(|scope| {
        let loaded = scope.spawn(|| {
            let loaded = set.load();
            tx.send(()).unwrap();
            loaded
        });
        assert!(
            rx.recv_timeout(BLOCKED).is_err(),
            "the sidecar was put back beside a read"
        );
        assert_eq!(std::fs::read(&set.store.sidecar).unwrap(), stale);
        drop(reader);
        rx.recv_timeout(FINISHES)
            .expect("the put-back ran once the read ended");
        let (store, pins) = loaded.join().unwrap().unwrap().unwrap();
        assert!(store.frozen);
        assert_eq!(pins, set.pins, "read as the pin binds it");
    });
    assert_eq!(set.store.read().unwrap().unwrap(), set.pins, "put back");
}

#[test]
fn a_read_nested_in_a_read_takes_the_filed_copy_and_writes_nothing() {
    let set = Frozen::new();
    let stale = set.behind();
    let _outer = PublishLockFile::hold_shared(&set.pin()).unwrap().unwrap();
    let (_, pins) = set.load().unwrap().unwrap();
    assert_eq!(pins, set.pins);
    assert_eq!(
        std::fs::read(&set.store.sidecar).unwrap(),
        stale,
        "a reader wrote the sidecar"
    );
}

#[test]
fn a_repin_over_pins_changed_since_they_were_read_writes_nothing() {
    let set = Frozen::new();
    let mut other = set.pins.clone();
    other.origin = "another repin".into();
    set.store.write(&other).unwrap();
    let mut mine = set.pins.clone();
    mine.origin = "mine".into();
    let pin_before = std::fs::read(set.pin()).unwrap();
    assert_eq!(
        set.store.write_over(&set.pins, &mine).unwrap(),
        Repin::Stale(other.clone())
    );
    assert_eq!(set.store.read().unwrap().unwrap(), other, "overwritten");
    assert_eq!(std::fs::read(set.pin()).unwrap(), pin_before);
    // Over the pins it now binds, the repin is written.
    assert_eq!(set.store.write_over(&other, &mine).unwrap(), Repin::Written);
    assert_eq!(set.store.verified_read().unwrap().unwrap(), mine);
    // A sidecar left behind its pin is judged by what the pin binds.
    set.behind();
    assert_eq!(set.store.write_over(&mine, &other).unwrap(), Repin::Written);
    assert_eq!(set.store.read().unwrap().unwrap(), other);
}

#[test]
fn a_repin_over_a_left_journal_with_no_settlement_is_unsettled_and_writes_nothing() {
    let set = Frozen::new();
    std::fs::write(&journal_paths(&set.pin())[0], br#"{"state": "applying"}"#).unwrap();
    let mut mine = set.pins.clone();
    mine.origin = "mine".into();
    let error = set.store.write_over(&set.pins, &mine).unwrap_err();
    let evidence = error.unsettled().expect("a left journal is unsettled");
    assert!(evidence.contains("state: applying"), "{evidence}");
    assert_eq!(set.store.read().unwrap().unwrap(), set.pins);
}
