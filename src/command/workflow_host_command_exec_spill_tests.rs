use super::workflow_host_command_catalog::fixed_decomposition_catalog;
use super::workflow_host_command_exec::{FixedHostCommandExecutor, WorkflowHostCommandExecutor};
use super::workflow_host_command_exec_tests::{PreparedBodyProcess, context, seed_frozen_chain};
use archon_workflow::HostCommandRequest;

async fn executor_publishes_prepared_body(overflow_stdout: bool) {
    let temp = tempfile::tempdir().unwrap();
    let mut context = context(temp.path());
    let task_file = context.task_root.join("TASK-X-010.md");
    std::fs::write(&task_file, b"live-before").unwrap();
    seed_frozen_chain(&context, &task_file);
    let store = archon_workflow::WorkflowStore::project(&context.project_root);
    let run = store
        .create_run(archon_workflow::WorkflowSpec {
            schema: archon_workflow::spec::WORKFLOW_SCHEMA.into(),
            name: "host-publication".into(),
            task: "test parent publication".into(),
            target_repository_root: None,
            max_parallelism: 1,
            max_agents: 1,
            stages: Vec::new(),
            permissions: Default::default(),
            learning_hooks: Vec::new(),
        })
        .unwrap();
    let run_root = store.run_dir(&run.id);
    context.run_staging_root = run_root.join("host-command-staging");
    let candidate = br#"# Candidate

```yaml
task_id: TASK-X-010
title: Candidate
complexity: low
status: ready
depends_on: []
blocks: []
implements: []
required_env_keys: []
required_tools: []
deliverable_contracts: []
```

## Focused Tests
- `test -f TASK-X-010.md`
"#
    .to_vec();
    let executor = FixedHostCommandExecutor::with_process(
        fixed_decomposition_catalog("rev-1").unwrap(),
        context,
        run_root.clone(),
        std::sync::Arc::new(PreparedBodyProcess {
            candidate: candidate.clone(),
            overflow_stdout,
        }),
    );
    let request = HostCommandRequest::new(
        "land-task-body",
        Some(String::from_utf8(candidate.clone()).unwrap()),
    )
    .unwrap();
    let call_id = executor.call_identity(&request).unwrap();
    let result = executor
        .execute(request, Some(run.generation))
        .await
        .unwrap();

    if !overflow_stdout {
        assert!(result.reusable(), "{result:?}");
    } else {
        assert!(
            result.stdout.contains("output truncated: "),
            "{}",
            result.stdout
        );
        assert!(
            result.stdout.contains("full output: host-command-results/"),
            "{}",
            result.stdout
        );
    }
    assert_eq!(std::fs::read(&task_file).unwrap(), candidate);
    let receipt = result.publication_receipt.unwrap();
    assert_eq!(receipt.call_id, call_id);
    assert_eq!(receipt.command_id, "land-task-body");
    assert_eq!(receipt.entries.len(), 2);
    let body = receipt
        .entries
        .iter()
        .find(|entry| entry.relative_path == "TASK-X-010.md")
        .unwrap();
    assert_eq!(
        body.blake3,
        archon_workflow::task_set_contract::content_digest(&candidate)
    );
    assert!(
        run_root
            .join("host-command-results")
            .join(call_id)
            .join("gate-envelope.json")
            .is_file()
    );
}

#[tokio::test]
async fn concrete_executor_audits_then_parent_publishes_exact_body_receipt() {
    executor_publishes_prepared_body(false).await;
}

#[tokio::test]
async fn executor_parses_prepared_manifest_from_full_spilled_stdout() {
    executor_publishes_prepared_body(true).await;
}
