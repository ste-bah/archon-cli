//! The `verify-frozen-*` capabilities through the concrete executor
//! (Issue-46): the staged child verifies the chain the freeze commands wrote,
//! the parent audits and publishes the envelope alone, and the postcondition
//! reads the frozen subjects exactly as `freeze-skeleton` would have.

use std::path::{Path, PathBuf};

use archon_workflow::HostCommandRequest;

use super::workflow_host_command_catalog::{
    HostCommandResolutionContext, ResolvedHostCommand, fixed_decomposition_catalog,
};
use super::workflow_host_command_exec::{FixedHostCommandExecutor, WorkflowHostCommandExecutor};

/// Runs the real staged child in-process: the resolved argv is parsed the way
/// the CLI would parse it and handed to `stage_verify`.
struct InProcessVerifyChild;

fn arg_after<'a>(args: &'a [String], flag: &str) -> &'a str {
    args.windows(2)
        .find(|pair| pair[0] == flag)
        .map(|pair| pair[1].as_str())
        .unwrap_or_else(|| panic!("{flag} in {args:?}"))
}

#[async_trait::async_trait]
impl super::workflow_host_command_exec::HostCommandProcessAdapter for InProcessVerifyChild {
    async fn execute(
        &self,
        request: ResolvedHostCommand,
        _control: super::workflow_host_command_supervisor::HostCommandControl,
    ) -> archon_workflow::WorkflowResult<
        super::workflow_host_command_supervisor::SupervisedProcessOutput,
    > {
        assert_eq!(&request.args[..2], ["workflow", "verify-frozen-chain"]);
        let manifest = super::workflow_decompose_frozen_chain::stage_verify(
            &request.cwd,
            arg_after(&request.args, "--stage"),
            Path::new(arg_after(&request.args, "--tasks")),
            Path::new(arg_after(&request.args, "--prd")),
            Some(Path::new(arg_after(&request.args, "--gate-envelope"))),
            Some(arg_after(&request.args, "--call-id")),
        )
        .unwrap();
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

fn frozen_context(root: &Path) -> (HostCommandResolutionContext, PathBuf) {
    let project_root = root.join("project");
    let task_root = project_root.join("tasks/PRD-X");
    let prd_path = project_root.join("prds/PRD-X.md");
    std::fs::create_dir_all(&task_root).unwrap();
    std::fs::create_dir_all(prd_path.parent().unwrap()).unwrap();
    std::fs::write(
        &prd_path,
        "# PRD X\n\n## Acceptance Criteria\n\n| ID | Criterion |\n|---|---|\n| AC-X-001 | The fixture is proven. |\n",
    )
    .unwrap();
    let project_root = project_root.canonicalize().unwrap();
    let task_root = task_root.canonicalize().unwrap();
    let prd_path = prd_path.canonicalize().unwrap();
    let context = HostCommandResolutionContext {
        program: PathBuf::from("/trusted/archon"),
        prd_digest: archon_workflow::task_set_contract::content_digest(
            &std::fs::read(&prd_path).unwrap(),
        ),
        project_root,
        prd_path,
        task_root: task_root.clone(),
        run_staging_root: root.join("run/staging"),
        frozen_task_id: None,
        frozen_task_file: None,
        freeze_provider_environment: Default::default(),
        gate_mode: archon_core::config::GateMode::Enforce,
    };
    (context, task_root)
}

#[tokio::test]
async fn the_executor_publishes_a_verified_frozen_chain_with_the_skeleton_subjects() {
    let temp = tempfile::tempdir().unwrap();
    let (mut context, task_root) = frozen_context(temp.path());
    super::workflow_decompose_frozen_chain::test_support::freeze_chain(
        &context.project_root,
        &context.prd_path,
        &task_root,
        &["TASK-X-010", "TASK-X-020"],
    );
    let store = archon_workflow::WorkflowStore::project(&context.project_root);
    let run = store
        .create_run(archon_workflow::WorkflowSpec {
            schema: archon_workflow::spec::WORKFLOW_SCHEMA.into(),
            name: "verify-frozen".into(),
            task: "verify a frozen chain".into(),
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
    let executor = FixedHostCommandExecutor::with_process(
        fixed_decomposition_catalog("rev-1").unwrap(),
        context.clone(),
        run_root.clone(),
        std::sync::Arc::new(InProcessVerifyChild),
    );

    for (capability, expected_subjects) in [
        ("verify-frozen-acceptance", 0usize),
        ("verify-frozen-skeleton", 2usize),
    ] {
        let request = HostCommandRequest::new(capability, None).unwrap();
        let call_id = executor.call_identity(&request).unwrap();
        let result = executor
            .execute(request, Some(run.generation))
            .await
            .unwrap();
        assert!(result.reusable(), "{capability}: {result:?}");
        let receipt = result.publication_receipt.as_ref().unwrap();
        assert_eq!(receipt.call_id, call_id);
        assert_eq!(receipt.command_id, capability);
        assert_eq!(
            receipt.entries.len(),
            1,
            "the envelope is the only publication"
        );
        assert!(
            receipt.entries[0]
                .destination_path
                .ends_with("gate-envelope.json")
        );
        assert_eq!(result.subjects.len(), expected_subjects, "{capability}");
        assert!(result.postcondition.as_ref().unwrap().satisfied);
    }
    // Nothing under the task root changed hands.
    let names: Vec<String> = std::fs::read_dir(&task_root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names.len(), 4, "{names:?}");
}

/// A chain the launcher would have refused is reported operationally through
/// the envelope; the parent publishes nothing and the outcome is not reusable.
#[tokio::test]
async fn a_frozen_chain_that_no_longer_verifies_is_an_operational_outcome_not_a_pass() {
    let temp = tempfile::tempdir().unwrap();
    let (mut context, task_root) = frozen_context(temp.path());
    super::workflow_decompose_frozen_chain::test_support::freeze_chain(
        &context.project_root,
        &context.prd_path,
        &task_root,
        &["TASK-X-010"],
    );
    std::fs::write(
        task_root.join(archon_workflow::task_set_contract::TASK_SKELETON_FILE),
        b"{}",
    )
    .unwrap();
    let store = archon_workflow::WorkflowStore::project(&context.project_root);
    let run = store
        .create_run(archon_workflow::WorkflowSpec {
            schema: archon_workflow::spec::WORKFLOW_SCHEMA.into(),
            name: "verify-frozen-broken".into(),
            task: "verify a broken chain".into(),
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
    let executor = FixedHostCommandExecutor::with_process(
        fixed_decomposition_catalog("rev-1").unwrap(),
        context,
        run_root,
        std::sync::Arc::new(InProcessVerifyChild),
    );
    let request = HostCommandRequest::new("verify-frozen-skeleton", None).unwrap();
    let result = executor
        .execute(request, Some(run.generation))
        .await
        .unwrap();
    assert!(result.publication_receipt.is_none());
    assert!(!result.reusable());
    let error = result
        .gate_envelope
        .unwrap()
        .operational_error
        .expect("operational error");
    assert!(error.text.contains("does not verify"), "{}", error.text);
}
