use super::super::test_baseline::tests::{Host, head, repository};
use super::*;
#[tokio::test]
async fn base_failures_without_a_file_never_become_task_obligations() {
    let temp = tempfile::tempdir().unwrap();
    let (root, worktree) = repository(temp.path());
    let base = head(&root);
    for (index, output) in ["opaque failure", "unavailable service", "missing.file:3:2"]
        .iter()
        .enumerate()
    {
        let store = WorkflowV2ResultStore::new(temp.path().join(format!("run-{index}/v2")));
        let ctx = WaveBaselineContext {
            repository_root: &root,
            store: &store,
            dispatch: &Host,
            universe: None,
            stage_id: "test",
            base_commit: &base,
            inputs_digest: None,
            parallelism: 1,
        };
        let command = format!("printf '{output}\\n'; exit 1");
        let records = establish_wave(
            &ctx,
            &[BranchBaselineRequest {
                branch_id: "branch".into(),
                task_ids: vec!["TASK-A".into()],
                commands: vec![command.clone()],
                worktree: worktree.clone(),
                targets: vec!["src/mine.rs".into()],
                forbidden: Default::default(),
            }],
        )
        .await;
        assert!(records[0].obligations.is_empty(), "{:?}", records[0]);
        assert_eq!(records[0].pre_existing.len(), 1);
        assert!(records[0].pre_existing[0].files.is_empty());
        assert!(
            super::super::test_baseline_preamble::preamble(&records[0])
                .contains("pre-existing, unattributed")
        );
    }
}
