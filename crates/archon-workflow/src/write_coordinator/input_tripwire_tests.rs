//! Batch G: the tripwire fails a call that changed the project's inputs,
//! puts the pre-call copies back, and never mistakes the host's own writes.
use super::*;
use crate::write_coordinator::project_inputs::write_test_policy;

struct Run {
    _dir: tempfile::TempDir,
    project: PathBuf,
    run_root: PathBuf,
    spec: PathBuf,
}

fn run() -> Run {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    let run_root = project.join(".archon/workflows/run1");
    std::fs::create_dir_all(&run_root).unwrap();
    std::fs::create_dir_all(project.join("tasks")).unwrap();
    let spec = project.join(".archon/lab/strategies/s1/strategy-spec.json");
    std::fs::create_dir_all(spec.parent().unwrap()).unwrap();
    std::fs::write(&spec, "{\"datasets\":[\"a\"]}").unwrap();
    std::fs::write(project.join("outside.txt"), "not an input").unwrap();
    write_test_policy(&run_root, &project, &[".archon/lab"]);
    let project = project
        .canonicalize()
        .map(archon_shell::paths::plain)
        .unwrap();
    let spec = project.join(".archon/lab/strategies/s1/strategy-spec.json");
    let run_root = run_root
        .canonicalize()
        .map(archon_shell::paths::plain)
        .unwrap();
    Run {
        _dir: dir,
        project,
        run_root,
        spec,
    }
}

/// The live shape: a read-only call regenerated a tracked input in place.
/// The call fails naming it, the copy is put back, the changed copy is kept
/// and the violation is logged under the run.
#[test]
fn a_call_that_rewrites_an_input_fails_and_the_host_puts_it_back() {
    let run = run();
    let tripwire = InputTripwire::arm(&run.run_root).expect("the run records inputs");
    std::fs::write(&run.spec, "{\"datasets\":[\"a\",\"b\"]}").unwrap();
    let created = run.project.join(".archon/lab/strategies/s1/new.json");
    std::fs::write(&created, "stray").unwrap();
    let violation = tripwire
        .check("verification-wave-review-verify-task-x-1-88")
        .expect("a violation");
    assert!(violation.restored(), "{violation:?}");
    assert_eq!(
        std::fs::read_to_string(&run.spec).unwrap(),
        "{\"datasets\":[\"a\"]}"
    );
    assert!(!created.exists(), "a file the call created is removed");
    let paths: Vec<&str> = violation.changed.iter().map(|c| c.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            ".archon/lab/strategies/s1/new.json",
            ".archon/lab/strategies/s1/strategy-spec.json"
        ]
    );
    let kept = violation
        .backup_dir
        .join(".archon/lab/strategies/s1/strategy-spec.json");
    assert_eq!(
        std::fs::read_to_string(kept).unwrap(),
        "{\"datasets\":[\"a\",\"b\"]}",
        "the changed copy is kept"
    );
    let message = violation.message();
    assert!(
        message.contains("ENVIRONMENT VIOLATION")
            && message.contains("verification-wave-review-verify-task-x-1-88"),
        "{message}"
    );
    let log = std::fs::read_to_string(
        run.run_root
            .join("write-coordination/environment-violations.jsonl"),
    )
    .unwrap();
    assert!(log.contains("strategy-spec.json"), "{log}");
}

/// Nothing changed, a file outside the inputs changed, or the host itself
/// wrote an input during the call: no violation.
#[test]
fn unchanged_inputs_outside_files_and_host_writes_are_not_violations() {
    let run = run();
    let tripwire = InputTripwire::arm(&run.run_root).unwrap();
    assert!(tripwire.check("quiet").is_none());

    let tripwire = InputTripwire::arm(&run.run_root).unwrap();
    std::fs::write(run.project.join("outside.txt"), "changed").unwrap();
    assert!(tripwire.check("outside").is_none());

    // A landing writes through `write_file`, which tells the tripwire.
    let tripwire = InputTripwire::arm(&run.run_root).unwrap();
    write_file(&run.project, &run.spec, b"{\"datasets\":[\"landed\"]}").unwrap();
    assert!(tripwire.check("during a landing").is_none());
    assert_eq!(
        std::fs::read_to_string(&run.spec).unwrap(),
        "{\"datasets\":[\"landed\"]}"
    );
}

/// A file the host wrote during the call and something else changed after
/// is a violation, and is NOT put back: that would undo the host's write.
#[test]
fn a_restore_never_undoes_a_host_write() {
    let run = run();
    let tripwire = InputTripwire::arm(&run.run_root).unwrap();
    write_file(&run.project, &run.spec, b"landed").unwrap();
    std::fs::write(&run.spec, "overwritten by the call").unwrap();
    let violation = tripwire.check("late writer").expect("a violation");
    assert!(!violation.restored());
    assert!(violation.changed[0].note.contains("host itself wrote"));
    assert_eq!(
        std::fs::read_to_string(&run.spec).unwrap(),
        "overwritten by the call"
    );
}

/// A write-capable call's own working tree is its work (a serial write in a
/// checkout that holds the inputs); anything else it changed is not.
#[test]
fn a_write_calls_own_tree_is_exempt_and_nothing_else_is() {
    let run = run();
    let tree = run.project.join(".archon/lab/strategies");
    let tripwire = InputTripwire::arm(&run.run_root).unwrap().exempting(&tree);
    std::fs::write(&run.spec, "its own work").unwrap();
    assert!(tripwire.check("serial writer").is_none());

    let other = run.project.join(".archon/lab/registry.json");
    std::fs::write(&other, "{}").unwrap();
    let tripwire = InputTripwire::arm(&run.run_root)
        .unwrap()
        .exempting(&run.project.join(".archon/lab/strategies/s1"));
    std::fs::write(&other, "{\"x\":1}").unwrap();
    let violation = tripwire.check("serial writer").expect("outside its tree");
    assert!(violation.restored());
    assert_eq!(std::fs::read_to_string(&other).unwrap(), "{}");
}

/// A run with no recorded inputs arms nothing.
#[test]
fn a_run_without_project_inputs_is_not_watched() {
    let dir = tempfile::tempdir().unwrap();
    let run_root = dir.path().join("project/.archon/workflows/run1");
    std::fs::create_dir_all(&run_root).unwrap();
    assert!(InputTripwire::arm(&run_root).is_none());
    let ((), violation) = watch_sync(Some(&run_root), "unwatched", || ());
    assert!(violation.is_none());
}

/// The async wrapper used around host-run commands reports and restores.
#[tokio::test]
async fn a_watched_host_command_that_writes_an_input_is_reported() {
    let run = run();
    let spec = run.spec.clone();
    let (out, violation) = watch(Some(&run.run_root), "host-run test command", async move {
        std::fs::write(&spec, "rewritten").unwrap();
        7
    })
    .await;
    assert_eq!(out, 7);
    assert!(violation.expect("reported").restored());
    assert_eq!(
        std::fs::read_to_string(&run.spec).unwrap(),
        "{\"datasets\":[\"a\"]}"
    );
}

/// Review M1: two calls overlap and one rewrites an input. Whichever is
/// checked first restores it; the other still fails, so the culprit never
/// passes on a file the host already put back.
#[test]
fn overlapping_calls_both_fail_whichever_is_checked_first() {
    let run = run();
    let innocent = InputTripwire::arm(&run.run_root).unwrap();
    let culprit = InputTripwire::arm(&run.run_root).unwrap();
    std::fs::write(&run.spec, "rewritten").unwrap();
    let first = innocent.check("overlap-innocent").expect("sees the change");
    assert!(first.restored());
    let second = culprit.check("overlap-culprit").expect("still failed");
    assert!(
        second.changed[0].note.contains("overlap-innocent"),
        "{second:?}"
    );
    // A call armed after the violation was handled is not failed by it.
    let later = InputTripwire::arm(&run.run_root).unwrap();
    assert!(later.check("overlap-later").is_none());
}

/// Review M3: the task set, the engine's own `.archon` files and toolchain
/// caches under an input are not inputs: the host (or an interpreter) writes
/// them, and they are never restored over.
#[test]
fn task_set_engine_files_and_toolchain_caches_are_not_judged() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    let run_root = project.join(".archon/workflows/run1");
    std::fs::create_dir_all(&run_root).unwrap();
    std::fs::create_dir_all(project.join("tasks")).unwrap();
    std::fs::create_dir_all(project.join(".archon/agents")).unwrap();
    std::fs::create_dir_all(project.join(".archon/lab/__pycache__")).unwrap();
    std::fs::write(project.join(".archon/archon-data.db"), "db").unwrap();
    std::fs::write(project.join(".archon/lab/data.json"), "{}").unwrap();
    write_test_policy(&run_root, &project, &[".archon", "tasks"]);
    let tripwire = InputTripwire::arm(&run_root).unwrap();
    std::fs::write(project.join(".archon/archon-data.db"), "db2").unwrap();
    std::fs::write(project.join(".archon/agents/a.md"), "x").unwrap();
    std::fs::write(project.join(".archon/lab/__pycache__/m.pyc"), "x").unwrap();
    std::fs::write(project.join("tasks/acceptance-contract.json"), "{}").unwrap();
    assert!(tripwire.check("host writes").is_none());
    let tripwire = InputTripwire::arm(&run_root).unwrap();
    std::fs::write(project.join(".archon/lab/data.json"), "{\"x\":1}").unwrap();
    assert!(tripwire.check("an input").is_some());
}

/// Re-check MED: a delivery an in-flight write call owns is left to that
/// call's own tripwire; a concurrent read-only call's check never restores
/// it from under the writer.
#[test]
fn an_in_flight_write_calls_delivery_is_not_restored_by_another_call() {
    let run = run();
    let reader = InputTripwire::arm(&run.run_root).unwrap();
    let writer = InFlight::register(std::slice::from_ref(&run.spec));
    std::fs::write(&run.spec, "delivered").unwrap();
    assert!(reader.check("concurrent reader").is_none());
    assert_eq!(std::fs::read_to_string(&run.spec).unwrap(), "delivered");
    drop(writer);
    let reader = InputTripwire::arm(&run.run_root).unwrap();
    std::fs::write(&run.spec, "rewritten after").unwrap();
    assert!(reader.check("reader after").is_some());
}

/// Issue-226: a call that writes an allowlisted external data directory
/// directly (not through a landing) is caught there too, and put back.
#[test]
fn a_direct_write_to_an_allowlisted_external_directory_is_caught_and_put_back() {
    let run = run();
    let allowed = run.project.parent().unwrap().join("allowed");
    std::fs::create_dir_all(allowed.join("lake")).unwrap();
    let allowed = allowed
        .canonicalize()
        .map(archon_shell::paths::plain)
        .unwrap();
    let bars = allowed.join("lake/bars.json");
    std::fs::write(&bars, "before").unwrap();
    let metadata_path = run.run_root.join("v2/generated-metadata.json");
    let mut metadata: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&metadata_path).unwrap()).unwrap();
    metadata["observer_snapshot"]["native_execution"]["external_data_roots"] =
        serde_json::json!([allowed]);
    std::fs::write(&metadata_path, serde_json::to_vec(&metadata).unwrap()).unwrap();
    let tripwire = InputTripwire::arm(&run.run_root).expect("armed");
    std::fs::write(&bars, "after").unwrap();
    let stray = allowed.join("lake/stray.json");
    std::fs::write(&stray, "stray").unwrap();
    let violation = tripwire.check("some-call").expect("a violation");
    assert!(violation.restored(), "{violation:?}");
    assert_eq!(std::fs::read_to_string(&bars).unwrap(), "before");
    assert!(!stray.exists());
    let kept = violation
        .backup_dir
        .join(super::super::project_inputs::external::stored_rel(
            bars.to_str().unwrap(),
        ));
    assert_eq!(std::fs::read_to_string(kept).unwrap(), "after");
    // The host's own write there is never one.
    let tripwire = InputTripwire::arm(&run.run_root).expect("armed");
    write_file(&allowed, &bars, b"landed").unwrap();
    assert!(tripwire.check("some-call").is_none());
}
