//! Issue 294: a reader inside a run that races a republish of the frozen
//! chain reads one whole version of it, never a mix of the old and the new.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use archon_workflow::task_set_contract::{
    ACCEPTANCE_CONTRACT_FILE, ACCEPTANCE_LOCK_FILE, AcceptanceCheck, AcceptanceContract,
    AcceptanceLock, TASK_SKELETON_FILE, TASK_SKELETON_LOCK_FILE, TrustedCwd, content_digest,
};

use crate::command::workflow_decompose_frozen_chain::test_support::{
    contract_bytes, freeze_chain, freeze_skeleton, stamp,
};
use crate::command::workflow_host_command_catalog::HostCommandResolutionContext;

/// Republications per race: enough that, without a consistent read, a mixed
/// read is seen on every run.
const REPUBLISHES: usize = 150;

const PRD: &str = "# PRD X\n\n## Acceptance Criteria\n\n| ID | Criterion |\n|---|---|\n| AC-X-001 | The fixture is proven. |\n";

struct Race {
    _temp: tempfile::TempDir,
    context: HostCommandResolutionContext,
    pin: PathBuf,
    /// The two complete versions the writer alternates between.
    versions: [Vec<(PathBuf, Vec<u8>)>; 2],
}

fn plain(path: &Path) -> PathBuf {
    path.canonicalize().map(archon_shell::paths::plain).unwrap()
}

/// A second whole chain: the one check re-authored, the skeleton re-bound.
fn freeze_reauthored(project: &Path, tasks: &Path, ids: &[&str]) {
    let mut contract: AcceptanceContract =
        serde_json::from_slice(&contract_bytes(&content_digest(PRD.as_bytes()))).unwrap();
    contract.acceptance[0].check = AcceptanceCheck::Command {
        command: "test -n reauthored".into(),
        cwd: TrustedCwd::ProjectRoot,
    };
    let bytes = serde_json::to_vec_pretty(&contract).unwrap();
    let digest = content_digest(&bytes);
    std::fs::write(tasks.join(ACCEPTANCE_CONTRACT_FILE), bytes).unwrap();
    let lock = AcceptanceLock {
        algorithm: "blake3".into(),
        digest: digest.clone(),
        gate: stamp(),
        baseline_commit: None,
    };
    std::fs::write(
        tasks.join(ACCEPTANCE_LOCK_FILE),
        serde_json::to_vec_pretty(&lock).unwrap(),
    )
    .unwrap();
    freeze_skeleton(project, tasks, &digest, ids);
}

fn race() -> Race {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let tasks = project.join("tasks/PRD-X");
    let prd = project.join("prds/PRD-X.md");
    std::fs::create_dir_all(&tasks).unwrap();
    std::fs::create_dir_all(prd.parent().unwrap()).unwrap();
    std::fs::write(&prd, PRD).unwrap();
    let (project, tasks, prd) = (plain(&project), plain(&tasks), plain(&prd));
    let pin = crate::command::workflow_task_set::acceptance_pin_path(&project, &tasks);
    let files = [
        tasks.join(ACCEPTANCE_CONTRACT_FILE),
        tasks.join(ACCEPTANCE_LOCK_FILE),
        tasks.join(TASK_SKELETON_FILE),
        tasks.join(TASK_SKELETON_LOCK_FILE),
        pin.clone(),
    ];
    let snapshot = || {
        files
            .iter()
            .map(|path| (path.clone(), std::fs::read(path).unwrap()))
            .collect::<Vec<_>>()
    };
    freeze_reauthored(&project, &tasks, &["TASK-X-010"]);
    let narrow = snapshot();
    freeze_chain(&project, &prd, &tasks, &["TASK-X-010", "TASK-X-020"]);
    let wide = snapshot();
    let context = HostCommandResolutionContext {
        program: PathBuf::from("/trusted/archon"),
        prd_digest: archon_workflow::task_set_contract::content_digest(PRD.as_bytes()),
        project_root: project,
        prd_path: prd,
        task_root: tasks,
        run_staging_root: temp.path().join("run/staging"),
        frozen_task_id: None,
        frozen_task_file: None,
        freeze_provider_environment: Default::default(),
        acceptance_environment_allowlist: Vec::new(),
        gate_mode: archon_core::config::GateMode::Enforce,
    };
    Race {
        _temp: temp,
        context,
        pin,
        versions: [wide, narrow],
    }
}

/// Republish the two versions in turn, as a per-check repair does (chain
/// lock, then one journaled transaction), while `read` runs in a loop on
/// another thread. Returns the mixed reads `read` reported and how many
/// reads ran.
fn run_race(
    read: impl Fn(&HostCommandResolutionContext) -> Result<(), String> + Send + 'static,
) -> (Vec<String>, usize) {
    let race = race();
    let done = Arc::new(AtomicBool::new(false));
    let reads = Arc::new(AtomicUsize::new(0));
    let reader = {
        let (done, reads, context) = (done.clone(), reads.clone(), race.context.clone());
        std::thread::spawn(move || {
            let mut mixed = Vec::new();
            while !done.load(Ordering::SeqCst) {
                if let Err(error) = read(&context) {
                    mixed.push(error);
                }
                reads.fetch_add(1, Ordering::SeqCst);
            }
            mixed
        })
    };
    let tasks = race.context.task_root.clone();
    for round in 0..REPUBLISHES {
        let _chain =
            crate::command::workflow_task_set::ChainLock::acquire_waiting(&race.pin, &tasks)
                .unwrap();
        crate::command::workflow_task_set::publish_files_atomically(
            &race.pin,
            &tasks,
            &race.versions[round % 2],
            "test republish",
        )
        .unwrap();
    }
    done.store(true, Ordering::SeqCst);
    let mixed = reader.join().unwrap();
    (mixed, reads.load(Ordering::SeqCst))
}

fn assert_never_mixed(name: &str, (mixed, reads): (Vec<String>, usize)) {
    assert!(
        reads > REPUBLISHES / 10,
        "{name}: the reader barely ran ({reads} reads)"
    );
    assert!(
        mixed.is_empty(),
        "{name}: {} of {reads} reads saw a mixed task set; first: {}",
        mixed.len(),
        mixed[0]
    );
}

#[test]
fn host_command_postcondition_racing_a_republish_never_reads_a_mixed_chain() {
    let result = run_race(|context| {
        let (subjects, evaluation) =
            crate::command::workflow_host_command_postcondition::evaluate_postcondition(
                context,
                "verify-frozen-skeleton",
            )
            .map_err(|error| error.to_string())?;
        match (evaluation.satisfied, subjects.len()) {
            (true, 1 | 2) => Ok(()),
            other => Err(format!("unexpected postcondition {other:?}")),
        }
    });
    assert_never_mixed("postcondition", result);
}

#[test]
fn frozen_acceptance_postcondition_racing_a_republish_never_reads_a_mixed_chain() {
    let result = run_race(|context| {
        let (_, evaluation) =
            crate::command::workflow_host_command_postcondition::evaluate_postcondition(
                context,
                "verify-frozen-acceptance",
            )
            .map_err(|error| error.to_string())?;
        if evaluation.satisfied {
            Ok(())
        } else {
            Err(format!("unsatisfied: {}", evaluation.summary))
        }
    });
    assert_never_mixed("acceptance postcondition", result);
}

const BLOCKED: std::time::Duration = std::time::Duration::from_millis(300);
const FINISHES: std::time::Duration = std::time::Duration::from_secs(20);

#[test]
fn a_consistent_read_holds_a_republish_back_only_until_it_ends() {
    let race = race();
    let tasks = race.context.task_root.clone();
    let read = crate::command::workflow_task_set::ChainRead::begin(&race.pin, &tasks).unwrap();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let (pin, files) = (race.pin.clone(), race.versions[1].clone());
    let writer = std::thread::spawn(move || {
        let _chain =
            crate::command::workflow_task_set::ChainLock::acquire_waiting(&pin, &tasks).unwrap();
        crate::command::workflow_task_set::publish_files_atomically(&pin, &tasks, &files, "test")
            .unwrap();
        done_tx.send(()).unwrap();
    });
    assert!(
        done_rx.recv_timeout(BLOCKED).is_err(),
        "the republish interleaved with the read"
    );
    drop(read);
    done_rx
        .recv_timeout(FINISHES)
        .expect("the republish ran once the read ended");
    writer.join().unwrap();
    assert_eq!(std::fs::read(&race.pin).unwrap(), race.versions[1][4].1);
}

#[test]
fn a_read_nested_in_a_publish_on_its_thread_reads_under_it_and_others_wait() {
    let race = race();
    let tasks = race.context.task_root.clone();
    let (pin, files) = (race.pin.clone(), race.versions[1].clone());
    let (nested_tx, nested_rx) = std::sync::mpsc::channel();
    let (commit_tx, commit_rx) = std::sync::mpsc::channel::<()>();
    let publisher = {
        let tasks = tasks.clone();
        std::thread::spawn(move || {
            let transaction =
                crate::command::workflow_task_set::begin_publish(&pin, &tasks, &files, "test", &[])
                    .unwrap();
            // The publisher verifying its own transaction must not wait on itself.
            let nested = crate::command::workflow_task_set::ChainRead::begin(&pin, &tasks);
            // A second publish on the same thread is refused, never a hang.
            let again =
                crate::command::workflow_task_set::begin_publish(&pin, &tasks, &files, "test", &[]);
            nested_tx.send(nested.is_ok() && again.is_err()).unwrap();
            drop((nested, again));
            commit_rx.recv().unwrap();
            transaction.commit().unwrap();
        })
    };
    assert!(
        nested_rx
            .recv_timeout(FINISHES)
            .expect("a nested read never deadlocks")
    );
    let (read_tx, read_rx) = std::sync::mpsc::channel();
    let (pin, other) = (race.pin.clone(), tasks.clone());
    let reader = std::thread::spawn(move || {
        let _read = crate::command::workflow_task_set::ChainRead::begin(&pin, &other).unwrap();
        read_tx.send(std::fs::read(&pin).unwrap()).unwrap();
    });
    assert!(
        read_rx.recv_timeout(BLOCKED).is_err(),
        "another thread read a half-applied set"
    );
    commit_tx.send(()).unwrap();
    let seen = read_rx
        .recv_timeout(FINISHES)
        .expect("the read ran after the commit");
    assert_eq!(
        seen, race.versions[1][4].1,
        "the read saw the whole new set"
    );
    publisher.join().unwrap();
    reader.join().unwrap();
}

#[test]
fn a_frozen_check_source_repin_waits_for_a_consistent_read() {
    use archon_workflow::check_source_pins::{CheckSourcePins, PinStore};
    let race = race();
    let (project, tasks) = (
        race.context.project_root.clone(),
        race.context.task_root.clone(),
    );
    let read = crate::command::workflow_task_set::ChainRead::begin(&race.pin, &tasks).unwrap();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let writer = std::thread::spawn(move || {
        let pins = CheckSourcePins {
            schema_version: 1,
            acceptance_digest: "digest".into(),
            origin: "test".into(),
            pinned_at: "2026-10-05T00:00:00Z".into(),
            checks: Default::default(),
            repins: Vec::new(),
        };
        done_tx
            .send(PinStore::frozen(&project, &tasks).write(&pins))
            .unwrap();
    });
    assert!(
        done_rx.recv_timeout(BLOCKED).is_err(),
        "the repin interleaved with the read"
    );
    drop(read);
    done_rx.recv_timeout(FINISHES).unwrap().unwrap();
    writer.join().unwrap();
    let pin: archon_workflow::task_set_contract::AcceptancePin =
        serde_json::from_slice(&std::fs::read(&race.pin).unwrap()).unwrap();
    assert!(
        pin.check_sources_digest.is_some(),
        "the repin recorded its sidecar"
    );
}

/// Issue 336: a run reading its check-source pins reads beside a consistent
/// read (both hold the lock shared) and waits only for a publish.
#[test]
fn a_run_reading_its_check_source_pins_reads_beside_a_read_and_waits_for_a_publish() {
    let race = race();
    let (project, tasks) = (
        race.context.project_root.clone(),
        race.context.task_root.clone(),
    );
    let run = race.context.run_staging_root.clone();
    std::fs::create_dir_all(&run).unwrap();
    let load = {
        let (project, tasks, run) = (project.clone(), tasks.clone(), run.clone());
        move || {
            let (done_tx, done_rx) = std::sync::mpsc::channel();
            let (project, tasks, run) = (project.clone(), tasks.clone(), run.clone());
            let reader = std::thread::spawn(move || {
                let roots = archon_workflow::check_source_resolve::Roots {
                    repository: &project,
                    project: &project,
                };
                let loaded = archon_workflow::check_source_pins::load_for_run(
                    &run, &project, &tasks, &roots,
                );
                done_tx.send(loaded.map(|loaded| loaded.is_some())).unwrap();
            });
            (reader, done_rx)
        }
    };
    let read = crate::command::workflow_task_set::ChainRead::begin(&race.pin, &tasks).unwrap();
    let (reader, done_rx) = load();
    assert!(
        done_rx
            .recv_timeout(FINISHES)
            .expect("the pins were held back by another reader")
            .unwrap()
    );
    reader.join().unwrap();
    drop(read);
    let publish = crate::command::workflow_task_set::begin_publish(
        &race.pin,
        &tasks,
        &race.versions[0],
        "test",
        &[],
    )
    .unwrap();
    let (reader, done_rx) = load();
    assert!(
        done_rx.recv_timeout(BLOCKED).is_err(),
        "the pins were read mid-publish"
    );
    publish.commit().unwrap();
    assert!(done_rx.recv_timeout(FINISHES).unwrap().unwrap());
    reader.join().unwrap();
}

/// Issue 336: two consistent reads hold the publish lock at once; neither
/// waits for the other.
#[test]
fn two_consistent_reads_hold_the_publish_lock_at_once() {
    let race = race();
    let tasks = race.context.task_root.clone();
    let first = crate::command::workflow_task_set::ChainRead::begin(&race.pin, &tasks).unwrap();
    let (held_tx, held_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let (pin, other) = (race.pin.clone(), tasks.clone());
    let second = std::thread::spawn(move || {
        let _read = crate::command::workflow_task_set::ChainRead::begin(&pin, &other).unwrap();
        held_tx.send(()).unwrap();
        release_rx.recv().unwrap();
    });
    held_rx
        .recv_timeout(FINISHES)
        .expect("the second read waited for the first: readers serialize");
    // Both hold it now; a writer waits for both.
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let (pin, files) = (race.pin.clone(), race.versions[1].clone());
    let writer = std::thread::spawn(move || {
        let _chain =
            crate::command::workflow_task_set::ChainLock::acquire_waiting(&pin, &tasks).unwrap();
        crate::command::workflow_task_set::publish_files_atomically(&pin, &tasks, &files, "test")
            .unwrap();
        done_tx.send(()).unwrap();
    });
    drop(first);
    assert!(
        done_rx.recv_timeout(BLOCKED).is_err(),
        "the republish ran while a read was held"
    );
    release_tx.send(()).unwrap();
    second.join().unwrap();
    done_rx
        .recv_timeout(FINISHES)
        .expect("the republish ran once both reads ended");
    writer.join().unwrap();
}

#[test]
fn a_read_of_a_project_that_never_froze_creates_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let tasks = temp.path().join("tasks/PRD-X");
    std::fs::create_dir_all(&tasks).unwrap();
    let read = crate::command::workflow_task_set::ChainRead::of(temp.path(), &tasks).unwrap();
    drop(read);
    assert!(
        !temp.path().join(".archon").exists(),
        "a reader created the pin store"
    );
}
