use super::super::test_baseline_wave::{WaveBaselineContext, establish_wave};
use super::tests::{Host, repository, request};
use crate::v2::WorkflowV2ResultStore;

#[tokio::test]
async fn baseline_wave_keeps_test_ids_for_declared_custom_commands() {
    let temp = tempfile::tempdir().unwrap();
    let (_, worktree) = repository(temp.path());
    let store = WorkflowV2ResultStore::new(temp.path().join("run/v2"));
    let command = "printf 'tests/test_example.py::test_one PASSED [100%%]\\n'";
    let ctx = WaveBaselineContext {
        store: &store,
        dispatch: &Host,
        universe: None,
        stage_id: "verification",
        base_commit: "fixture",
        inputs_digest: None,
        parallelism: 1,
    };
    let records = establish_wave(&ctx, &[request("branch", "task", command, &worktree, &[])]).await;
    assert_eq!(
        records[0].commands[0].passed_tests,
        ["tests/test_example.py::test_one"]
    );
    let cached = establish_wave(&ctx, &[request("branch", "task", command, &worktree, &[])]).await;
    assert!(cached[0].commands[0].cached);
    assert_eq!(
        cached[0].commands[0].passed_tests,
        records[0].commands[0].passed_tests
    );
}
