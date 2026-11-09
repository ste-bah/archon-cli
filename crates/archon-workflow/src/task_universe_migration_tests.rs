//! Legacy decisions become durable before the normal regression comparison.
use super::*;
use crate::v2::verification::regression_gate::{RegressionGate, regression_verdict};
use crate::v2::write::test_baseline_run_base::tests::{bind_run, freeze_runner_clock, head, world};
use crate::write_coordinator::worktree_isolation::run_git;
use std::io::Write;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
struct Log(Arc<Mutex<Vec<u8>>>);
impl Write for Log {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn task_universe_legacy_migration_is_logged_persisted_and_judged_normally() {
    let _clock = freeze_runner_clock();
    let temp = tempfile::tempdir().unwrap();
    let (repo, _, host, _) = world(temp.path());
    std::fs::write(repo.join("state"), "old\n").unwrap();
    run_git(&["add", "."], &repo).unwrap();
    run_git(&["commit", "-qm", "base failure"], &repo).unwrap();
    let base = head(&repo);
    let command = "printf 'error: '; cat state; exit 1";
    let mut universe = WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: vec![],
        tasks: vec![WorkflowV2TaskUniverseTask {
            canonical_task_id: "TASK-A".into(),
            focused_tests: vec![command.into()],
            required_tools: vec!["printf".into()],
            ..Default::default()
        }],
    };
    assert!(!universe.command_decisions_are_recorded());
    let log = Log(Arc::new(Mutex::new(Vec::new())));
    let captured = log.clone();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(move || captured.clone())
        .finish();
    assert!(tracing::subscriber::with_default(subscriber, || universe
        .migrate_command_decisions(&repo, std::ffi::OsStr::new(""))));
    assert!(
        String::from_utf8(log.0.lock().unwrap().clone())
            .unwrap()
            .contains("migrated frozen command decisions")
    );
    let path = temp.path().join("universe.json");
    std::fs::write(&path, serde_json::to_vec(&universe).unwrap()).unwrap();
    let mut universe: WorkflowV2TaskUniverse =
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert!(!universe.migrate_command_decisions(&repo, std::ffi::OsStr::new("unavailable")));
    assert_eq!(
        universe.tasks[0].declared_focused_test_commands(),
        [command]
    );
    let store = crate::v2::WorkflowV2ResultStore::new(temp.path().join("run/v2"));
    bind_run(&store, &base);
    std::fs::write(repo.join("unrelated"), "tip\n").unwrap();
    run_git(&["add", "."], &repo).unwrap();
    run_git(&["commit", "-qm", "unchanged failure"], &repo).unwrap();
    let gate = RegressionGate {
        store: &store,
        dispatch: &host,
        universe: Some(&universe),
        repository_root: &repo,
    };
    let same = regression_verdict(&gate).await;
    assert!(same.blocking.is_empty(), "{same:?}");
    assert!(
        same.notes
            .iter()
            .any(|note| note.starts_with("PRE-EXISTING")),
        "{same:?}"
    );
    std::fs::write(repo.join("state"), "new\n").unwrap();
    run_git(&["add", "."], &repo).unwrap();
    run_git(&["commit", "-qm", "different failure"], &repo).unwrap();
    let different = regression_verdict(&gate).await;
    assert!(!different.blocking.is_empty(), "{different:?}");
}
