//! Concrete catalog→process→audit→parent-publication integration tests.

use std::path::PathBuf;

use archon_workflow::HostCommandRequest;

use super::workflow_host_command_catalog::{
    HostCommandResolutionContext, fixed_decomposition_catalog,
};

fn context(root: &std::path::Path) -> HostCommandResolutionContext {
    let project_root = root.join("project");
    let task_root = project_root.join("tasks/PRD-X");
    let prd_path = project_root.join("prds/PRD-X.md");
    std::fs::create_dir_all(&task_root).unwrap();
    std::fs::create_dir_all(prd_path.parent().unwrap()).unwrap();
    std::fs::write(&prd_path, "# PRD\n").unwrap();
    HostCommandResolutionContext {
        program: PathBuf::from("/trusted/archon"),
        project_root,
        prd_digest: archon_workflow::task_set_contract::content_digest(
            &std::fs::read(&prd_path).unwrap(),
        ),
        prd_path,
        task_root,
        run_staging_root: root.join("run/staging"),
        frozen_task_id: None,
        frozen_task_file: None,
        freeze_provider_environment: Default::default(),
        gate_mode: archon_core::config::GateMode::Observe,
    }
}

fn seed_frozen_chain(context: &HostCommandResolutionContext, task_file: &std::path::Path) {
    use archon_workflow::task_set_contract::{
        ACCEPTANCE_CONTRACT_FILE, ACCEPTANCE_LOCK_FILE, AcceptanceLock, AcceptancePin,
        FreezeGateMode, FreezeGateStamp, TASK_SKELETON_FILE, TASK_SKELETON_LOCK_FILE,
        content_digest, empty_gate_findings_digest,
    };
    use archon_workflow::task_skeleton::{FrozenTask, TaskSkeleton, TaskSkeletonLock};

    let stamp = || FreezeGateStamp {
        mode: FreezeGateMode::Enforce,
        finding_count: 0,
        findings_digest: empty_gate_findings_digest(),
        binary_commit: "rev-1".into(),
        evaluated_at: "2026-08-27T00:00:00Z".into(),
    };
    let acceptance = b"{}";
    std::fs::write(context.task_root.join(ACCEPTANCE_CONTRACT_FILE), acceptance).unwrap();
    let acceptance_digest = content_digest(acceptance);
    std::fs::write(
        context.task_root.join(ACCEPTANCE_LOCK_FILE),
        serde_json::to_vec_pretty(&AcceptanceLock {
            algorithm: "blake3".into(),
            digest: acceptance_digest.clone(),
            gate: stamp(),
        })
        .unwrap(),
    )
    .unwrap();
    let skeleton = TaskSkeleton {
        schema_version: 1,
        acceptance_digest: acceptance_digest.clone(),
        tasks: vec![FrozenTask {
            task_id: "TASK-X-010".into(),
            file_name: task_file
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            depends_on: Vec::new(),
            blocks: Vec::new(),
            implements: Vec::new(),
            deliverable_contracts: Vec::new(),
        }],
    };
    let skeleton_bytes = serde_json::to_vec_pretty(&skeleton).unwrap();
    let skeleton_digest = content_digest(&skeleton_bytes);
    std::fs::write(context.task_root.join(TASK_SKELETON_FILE), skeleton_bytes).unwrap();
    std::fs::write(
        context.task_root.join(TASK_SKELETON_LOCK_FILE),
        serde_json::to_vec_pretty(&TaskSkeletonLock {
            algorithm: "blake3".into(),
            digest: skeleton_digest.clone(),
            acceptance_digest: acceptance_digest.clone(),
            gate: stamp(),
        })
        .unwrap(),
    )
    .unwrap();
    let pin_path =
        super::workflow_task_set::acceptance_pin_path(&context.project_root, &context.task_root);
    std::fs::create_dir_all(pin_path.parent().unwrap()).unwrap();
    std::fs::write(
        pin_path,
        serde_json::to_vec_pretty(&AcceptancePin {
            task_root: context
                .task_root
                .canonicalize()
                .unwrap()
                .display()
                .to_string(),
            acceptance_digest,
            freeze_event_id: "acceptance-freeze-fixture".into(),
            acceptance_gate: stamp(),
            skeleton_digest: Some(skeleton_digest),
            skeleton_gate: Some(stamp()),
        })
        .unwrap(),
    )
    .unwrap();
}

struct PreparedBodyProcess {
    candidate: Vec<u8>,
}

#[async_trait::async_trait]
impl super::workflow_host_command_exec::HostCommandProcessAdapter for PreparedBodyProcess {
    async fn execute(
        &self,
        request: super::workflow_host_command_catalog::ResolvedHostCommand,
        _control: super::workflow_host_command_supervisor::HostCommandControl,
    ) -> archon_workflow::WorkflowResult<
        super::workflow_host_command_supervisor::SupervisedProcessOutput,
    > {
        use archon_workflow::{
            GATE_ENVELOPE_SCHEMA_VERSION, GateEnvelopeV1, PREPARED_PUBLICATION_SCHEMA_VERSION,
            PreparedPublicationEntry, PreparedPublicationV1,
        };

        let envelope = GateEnvelopeV1 {
            schema_version: GATE_ENVELOPE_SCHEMA_VERSION,
            report: serde_json::json!("body accepted"),
            policy_findings: Vec::new(),
            operational_error: None,
        };
        let envelope_bytes = serde_json::to_vec_pretty(&envelope).unwrap();
        let mut entries = Vec::new();
        for path in &request.declared_write_set {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            let bytes = if path.file_name().unwrap() == "gate-envelope.json" {
                envelope_bytes.clone()
            } else {
                self.candidate.clone()
            };
            std::fs::write(path, &bytes).unwrap();
            entries.push(PreparedPublicationEntry {
                relative_path: path.file_name().unwrap().to_string_lossy().into_owned(),
                byte_len: bytes.len() as u64,
                blake3: archon_workflow::task_set_contract::content_digest(&bytes),
            });
        }
        entries.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
        let call_id = request
            .args
            .windows(2)
            .find(|pair| pair[0] == "--call-id")
            .map(|pair| pair[1].clone())
            .unwrap();
        let manifest = PreparedPublicationV1 {
            schema_version: PREPARED_PUBLICATION_SCHEMA_VERSION,
            call_id,
            command_id: request.command_id,
            entries,
        };
        let stdout = serde_json::to_vec(&manifest).unwrap();
        Ok(
            super::workflow_host_command_supervisor::SupervisedProcessOutput {
                exit_code: Some(0),
                stdout_bytes: stdout.len() as u64,
                stderr_bytes: 0,
                stdout,
                stderr: Vec::new(),
            },
        )
    }
}

#[tokio::test]
async fn concrete_executor_audits_then_parent_publishes_exact_body_receipt() {
    use super::workflow_host_command_exec::{
        FixedHostCommandExecutor, WorkflowHostCommandExecutor,
    };

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

    assert!(result.reusable());
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

struct ControlWaitingProcess {
    started: std::sync::Arc<tokio::sync::Notify>,
}

#[async_trait::async_trait]
impl super::workflow_host_command_exec::HostCommandProcessAdapter for ControlWaitingProcess {
    async fn execute(
        &self,
        request: super::workflow_host_command_catalog::ResolvedHostCommand,
        control: super::workflow_host_command_supervisor::HostCommandControl,
    ) -> archon_workflow::WorkflowResult<
        super::workflow_host_command_supervisor::SupervisedProcessOutput,
    > {
        self.started.notify_one();
        Err(match control.wait().await {
            super::workflow_host_command_supervisor::HostCommandSignal::Paused => {
                archon_workflow::WorkflowError::ControlPaused(format!(
                    "host command '{}' paused while in flight",
                    request.command_id
                ))
            }
            super::workflow_host_command_supervisor::HostCommandSignal::Cancelled => {
                archon_workflow::WorkflowError::ControlCancelled(format!(
                    "host command '{}' cancelled while in flight",
                    request.command_id
                ))
            }
        })
    }
}

#[tokio::test]
async fn fixed_host_command_pause_signals_inflight_supervisor() {
    use super::workflow_host_command_exec::{
        FixedHostCommandExecutor, WorkflowHostCommandExecutor,
    };

    let temp = tempfile::tempdir().unwrap();
    let context = context(temp.path());
    let store = archon_workflow::WorkflowStore::project(&context.project_root);
    let run = store
        .create_run(archon_workflow::WorkflowSpec {
            schema: archon_workflow::spec::WORKFLOW_SCHEMA.into(),
            name: "host-control".into(),
            task: "test".into(),
            target_repository_root: None,
            max_parallelism: 1,
            max_agents: 1,
            stages: Vec::new(),
            permissions: Default::default(),
            learning_hooks: Vec::new(),
        })
        .unwrap();
    let started = std::sync::Arc::new(tokio::sync::Notify::new());
    let executor = std::sync::Arc::new(FixedHostCommandExecutor::with_process(
        fixed_decomposition_catalog("rev-1").unwrap(),
        context,
        store.run_dir(&run.id),
        std::sync::Arc::new(ControlWaitingProcess {
            started: std::sync::Arc::clone(&started),
        }),
    ));
    let generation = run.generation;
    let task = tokio::spawn(async move {
        executor
            .execute(
                HostCommandRequest::new("task-set-lint", None).unwrap(),
                Some(generation),
            )
            .await
    });

    tokio::time::timeout(std::time::Duration::from_secs(2), started.notified())
        .await
        .expect("HostCommand process must start before pause");
    archon_workflow::LifecycleController::new(store)
        .apply(&run.id, archon_workflow::LifecycleAction::Pause)
        .unwrap();
    let error = tokio::time::timeout(std::time::Duration::from_secs(2), task)
        .await
        .expect("HostCommand pause must reach supervisor")
        .unwrap()
        .unwrap_err();

    assert!(matches!(
        error,
        archon_workflow::WorkflowError::ControlPaused(_)
    ));
}

#[tokio::test]
async fn fixed_host_command_pause_then_resume_still_cancels_old_process_generation() {
    use super::workflow_host_command_exec::{
        FixedHostCommandExecutor, WorkflowHostCommandExecutor,
    };

    let temp = tempfile::tempdir().unwrap();
    let context = context(temp.path());
    let store = archon_workflow::WorkflowStore::project(&context.project_root);
    let run = store
        .create_run(archon_workflow::WorkflowSpec {
            schema: archon_workflow::spec::WORKFLOW_SCHEMA.into(),
            name: "host-generation-control".into(),
            task: "test".into(),
            target_repository_root: None,
            max_parallelism: 1,
            max_agents: 1,
            stages: Vec::new(),
            permissions: Default::default(),
            learning_hooks: Vec::new(),
        })
        .unwrap();
    let started = std::sync::Arc::new(tokio::sync::Notify::new());
    let executor = std::sync::Arc::new(FixedHostCommandExecutor::with_process(
        fixed_decomposition_catalog("rev-1").unwrap(),
        context,
        store.run_dir(&run.id),
        std::sync::Arc::new(ControlWaitingProcess {
            started: std::sync::Arc::clone(&started),
        }),
    ));
    let generation = run.generation;
    let task = tokio::spawn(async move {
        executor
            .execute(
                HostCommandRequest::new("task-set-lint", None).unwrap(),
                Some(generation),
            )
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(2), started.notified())
        .await
        .expect("HostCommand process must start before lifecycle advance");

    let lifecycle = archon_workflow::LifecycleController::new(store.clone());
    lifecycle
        .apply(&run.id, archon_workflow::LifecycleAction::Pause)
        .unwrap();
    lifecycle
        .apply(&run.id, archon_workflow::LifecycleAction::Resume)
        .unwrap();
    let error = tokio::time::timeout(std::time::Duration::from_secs(2), task)
        .await
        .expect("old HostCommand generation must stop")
        .unwrap()
        .unwrap_err();

    assert!(matches!(
        error,
        archon_workflow::WorkflowError::ControlCancelled(_)
    ));
    assert_eq!(
        store.load_state(&run.id).unwrap().status,
        archon_workflow::RunStatus::Running
    );
}
