use super::*;

/// The launch reads the frozen chain, rehearses the script against it (the
/// dry run answers `verify-frozen-skeleton` with a stand-in subject) and
/// binds the reading into the persisted arguments.
struct ArgumentsBarrier {
    project: PathBuf,
    seen: std::sync::Mutex<Option<serde_json::Value>>,
}

#[async_trait::async_trait(?Send)]
impl archon_workflow::WorkflowLlmClientFactory for ArgumentsBarrier {
    async fn build_client(
        &self,
        _request: archon_workflow::WorkflowLlmClientRequest,
    ) -> archon_workflow::WorkflowResult<std::sync::Arc<dyn archon_workflow::WorkflowLlmClient>>
    {
        let store = archon_workflow::WorkflowStore::project(&self.project);
        let run = store.list_runs()?.pop().expect("one run");
        let path = store
            .run_dir(&run.id)
            .join(crate::command::workflow_decompose::FIXED_ARGUMENTS_PATH);
        let arguments: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        *self.seen.lock().unwrap() = Some(arguments);
        Err(archon_workflow::WorkflowError::port("barrier observed"))
    }
}

#[tokio::test]
async fn a_launch_on_a_frozen_task_root_binds_the_frozen_chain_into_its_arguments() {
    let mut fixture = project();
    freeze_acceptance(&mut fixture);
    freeze_skeleton(&fixture);
    std::fs::write(fixture.tasks.join("TASK-X-010.md"), "# body").unwrap();
    let factory = ArgumentsBarrier {
        project: fixture.project.clone(),
        seen: std::sync::Mutex::new(None),
    };
    let error = crate::command::workflow_decompose::run_fixed_decomposition_with_factory(
        &fixture.project,
        Path::new("prds/PRD-X.md"),
        Path::new("tasks/PRD-X"),
        None,
        true,
        &launch_config(&fixture.project),
        &archon_core::env_vars::load_env_vars_from(&std::collections::HashMap::new()),
        &factory,
    )
    .await
    .unwrap_err();
    assert!(
        format!("{error:#}").contains("barrier observed"),
        "{error:#}"
    );
    let seen = factory
        .seen
        .lock()
        .unwrap()
        .clone()
        .expect("arguments persisted");
    assert_eq!(
        seen["frozenChain"],
        serde_json::json!({
            "acceptance": true,
            "skeleton": true,
            "subjects": [
                {"taskId": "TASK-X-010", "fileName": "TASK-X-010.md"},
                {"taskId": "TASK-X-020", "fileName": "TASK-X-020.md"},
            ],
            "bodies": ["TASK-X-010.md"],
        })
    );
}

#[tokio::test]
async fn a_launch_on_a_task_root_whose_lock_does_not_verify_creates_no_run() {
    let mut fixture = project();
    freeze_acceptance(&mut fixture);
    std::fs::write(
        fixture.tasks.join(ACCEPTANCE_CONTRACT_FILE),
        contract_bytes("edited"),
    )
    .unwrap();
    let factory = ArgumentsBarrier {
        project: fixture.project.clone(),
        seen: std::sync::Mutex::new(None),
    };
    let error = crate::command::workflow_decompose::run_fixed_decomposition_with_factory(
        &fixture.project,
        Path::new("prds/PRD-X.md"),
        Path::new("tasks/PRD-X"),
        None,
        true,
        &launch_config(&fixture.project),
        &archon_core::env_vars::load_env_vars_from(&std::collections::HashMap::new()),
        &factory,
    )
    .await
    .unwrap_err();
    assert!(
        format!("{error:#}").contains("does not verify"),
        "{error:#}"
    );
    assert!(factory.seen.lock().unwrap().is_none());
    assert!(!fixture.project.join(".archon/workflows").exists());
}
