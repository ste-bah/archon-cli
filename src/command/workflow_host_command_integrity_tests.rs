use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use archon_workflow::HostCommandRequest;

use super::workflow_host_command_catalog::{
    HostCommandResolutionContext, ResolvedHostCommand, fixed_decomposition_catalog,
};
use super::workflow_host_command_exec::{
    FixedHostCommandExecutor, HostCommandProcessAdapter, WorkflowHostCommandExecutor,
};
use super::workflow_host_command_supervisor::{HostCommandControl, SupervisedProcessOutput};

struct MustNotStartProcess {
    calls: AtomicUsize,
}

#[async_trait::async_trait]
impl HostCommandProcessAdapter for MustNotStartProcess {
    async fn execute(
        &self,
        _request: ResolvedHostCommand,
        _control: HostCommandControl,
    ) -> archon_workflow::WorkflowResult<SupervisedProcessOutput> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        panic!("mutated launch PRD must be refused before process execution")
    }
}

#[tokio::test]
async fn initial_acceptance_freeze_refuses_prd_changed_after_launch_digest() {
    let temp = tempfile::tempdir().unwrap();
    let project_root = temp.path().join("project");
    let task_root = project_root.join("tasks/PRD-X");
    let prd_path = project_root.join("prds/PRD-X.md");
    std::fs::create_dir_all(&task_root).unwrap();
    std::fs::create_dir_all(prd_path.parent().unwrap()).unwrap();
    std::fs::write(
        &prd_path,
        "# PRD X\n\n## Requirements\n| ID | Requirement |\n|---|---|\n| REQ-X-001 | Initial |\n\n## Acceptance\n| ID | Criterion |\n|---|---|\n| AC-X-001 | Initial |\n",
    )
    .unwrap();
    let (_, prd_digest, _) = super::workflow_task_set::validate_prd_input(&prd_path).unwrap();
    let store = archon_workflow::WorkflowStore::project(&project_root);
    let run = store
        .create_run(archon_workflow::WorkflowSpec {
            schema: archon_workflow::spec::WORKFLOW_SCHEMA.into(),
            name: "prd-integrity".into(),
            task: "freeze immutable PRD".into(),
            target_repository_root: None,
            max_parallelism: 1,
            max_agents: 1,
            stages: Vec::new(),
            permissions: Default::default(),
            learning_hooks: Vec::new(),
        })
        .unwrap();
    let process = Arc::new(MustNotStartProcess {
        calls: AtomicUsize::new(0),
    });
    let executor = FixedHostCommandExecutor::with_process(
        fixed_decomposition_catalog("revision").unwrap(),
        HostCommandResolutionContext {
            program: PathBuf::from("/trusted/archon"),
            project_root,
            prd_path: prd_path.clone(),
            prd_digest,
            task_root: task_root.clone(),
            run_staging_root: store.run_dir(&run.id).join("host-command-staging"),
            frozen_task_id: None,
            frozen_task_file: None,
            freeze_provider_environment: Default::default(),
            gate_mode: archon_core::config::GateMode::Observe,
        },
        store.run_dir(&run.id),
        process.clone(),
    );
    std::fs::write(
        &prd_path,
        "# PRD X\n\n## Requirements\n| ID | Requirement |\n|---|---|\n| REQ-X-001 | Changed |\n\n## Acceptance\n| ID | Criterion |\n|---|---|\n| AC-X-001 | Changed |\n",
    )
    .unwrap();

    let error = executor
        .execute(
            HostCommandRequest::new("freeze-acceptance", Some("candidate".into())).unwrap(),
            Some(run.generation),
        )
        .await
        .unwrap_err()
        .to_string();

    assert!(error.contains("launch PRD digest"), "{error}");
    assert_eq!(process.calls.load(Ordering::SeqCst), 0);
    assert!(!task_root.join("acceptance-contract.lock").exists());
}
