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

#[cfg(unix)]
#[tokio::test]
async fn executor_parses_prepared_manifest_from_full_spilled_stdout() {
    executor_publishes_prepared_body(true).await;
}

/// Issue 386 (item 3): a process that reads its candidate the way the trusted
/// child does, from stdin or the spill a bounded view names.
struct SpilledStdinProcess {
    inner: PreparedBodyProcess,
    seen: std::sync::Arc<std::sync::Mutex<Option<(Vec<u8>, Vec<u8>)>>>,
}

#[async_trait::async_trait]
impl super::workflow_host_command_exec::HostCommandProcessAdapter for SpilledStdinProcess {
    async fn execute(
        &self,
        request: super::workflow_host_command_catalog::ResolvedHostCommand,
        control: super::workflow_host_command_supervisor::HostCommandControl,
    ) -> archon_workflow::WorkflowResult<
        super::workflow_host_command_supervisor::SupervisedProcessOutput,
    > {
        let view = request.stdin.clone().unwrap();
        let staging = request
            .args
            .windows(2)
            .find(|pair| pair[0] == "--staging-root")
            .map(|pair| std::path::PathBuf::from(&pair[1]))
            .unwrap();
        let full = super::workflow_host_command_stdin_spill::read_candidate(
            view.as_slice(),
            HostCommandRequest::MAX_STDIN_BYTES,
            &staging,
        )
        .unwrap();
        *self.seen.lock().unwrap() = Some((view, full));
        self.inner.execute(request, control).await
    }
}

#[tokio::test]
async fn an_oversized_candidate_reaches_the_child_whole_through_a_run_spill() {
    let temp = tempfile::tempdir().unwrap();
    let mut context = context(temp.path());
    let task_file = context.task_root.join("TASK-X-010.md");
    std::fs::write(&task_file, b"live-before").unwrap();
    seed_frozen_chain(&context, &task_file);
    let store = archon_workflow::WorkflowStore::project(&context.project_root);
    let run = store
        .create_run(archon_workflow::WorkflowSpec {
            schema: archon_workflow::spec::WORKFLOW_SCHEMA.into(),
            name: "host-stdin-spill".into(),
            task: "test stdin spill".into(),
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
    let catalog = fixed_decomposition_catalog("rev-1").unwrap();
    let cap = catalog.capabilities["land-task-body"].max_stdin_bytes as usize;
    let mut candidate = br#"# Candidate

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

## Notes
"#
    .to_vec();
    while candidate.len() <= cap {
        candidate.extend_from_slice("- détail répété pour dépasser la borne\n".as_bytes());
    }
    let seen = std::sync::Arc::new(std::sync::Mutex::new(None));
    let executor = FixedHostCommandExecutor::with_process(
        catalog,
        context,
        run_root.clone(),
        std::sync::Arc::new(SpilledStdinProcess {
            inner: PreparedBodyProcess {
                candidate: candidate.clone(),
                overflow_stdout: false,
            },
            seen: seen.clone(),
        }),
    );
    let request = HostCommandRequest::new(
        "land-task-body",
        Some(String::from_utf8(candidate.clone()).unwrap()),
    )
    .expect("an oversized candidate is not refused at the bridge");
    let call_id = executor.call_identity(&request).unwrap();
    let result = executor
        .execute(request, Some(run.generation))
        .await
        .expect("an oversized candidate must not fail the call");
    assert!(result.reusable(), "{result:?}");
    let (view, full) = seen.lock().unwrap().take().unwrap();
    assert!(view.len() < 4096, "the child's stdin is a bounded view");
    assert!(
        view.starts_with(super::workflow_host_command_stdin_spill::STDIN_SPILL_PREFIX.as_bytes())
    );
    // The host stamps frozen frontmatter fields into the candidate before
    // it runs (#428); the child reads every byte of that, never the view.
    assert!(full.len() > cap, "{} bytes", full.len());
    assert!(
        full.ends_with(
            b"- d\xc3\xa9tail r\xc3\xa9p\xc3\xa9t\xc3\xa9 pour d\xc3\xa9passer la borne\n"
        )
    );
    let spill = run_root.join("host-command-results").join(&call_id);
    let spilled: Vec<_> = std::fs::read_dir(&spill)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path().join("stdin.bin"))
        .filter(|path| path.is_file())
        .collect();
    assert_eq!(spilled.len(), 1, "one spill under {}", spill.display());
    assert_eq!(std::fs::read(&spilled[0]).unwrap(), full);
    assert_eq!(std::fs::read(&task_file).unwrap(), candidate);
}
