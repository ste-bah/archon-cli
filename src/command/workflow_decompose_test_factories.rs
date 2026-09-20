//! Provider factories and sinks shared by the fixed-decomposition tests: each
//! one is a barrier that observes the run's persisted state at the moment the
//! launcher asks for a provider client.

use super::*;

pub(super) struct BarrierFactory {
    pub(super) project_root: PathBuf,
    pub(super) builds: AtomicUsize,
    pub(super) expected_status: RunStatus,
}

impl BarrierFactory {
    pub(super) fn launch(project_root: PathBuf) -> Self {
        Self {
            project_root,
            builds: AtomicUsize::new(0),
            expected_status: RunStatus::Planned,
        }
    }

    pub(super) fn resume(project_root: PathBuf) -> Self {
        Self {
            project_root,
            builds: AtomicUsize::new(0),
            expected_status: RunStatus::Paused,
        }
    }
}

#[async_trait::async_trait(?Send)]
impl WorkflowLlmClientFactory for BarrierFactory {
    async fn build_client(
        &self,
        request: WorkflowLlmClientRequest,
    ) -> archon_workflow::WorkflowResult<Arc<dyn WorkflowLlmClient>> {
        self.builds.fetch_add(1, Ordering::SeqCst);
        let store = WorkflowStore::project(&self.project_root);
        let runs = store.list_runs()?;
        assert_eq!(runs.len(), 1, "one run must exist before provider build");
        let run = &runs[0];
        assert_eq!(run.status, self.expected_status);
        assert_eq!(request.session_id, run.id);
        assert!(crate::command::workflow_task_root_reclaim::begin_execution(&store, &run.id).is_err(),
            "launch/resume must hold an execution lease before provider construction");
        assert_eq!(request.origin, "workflow_decompose_v1");

        let fixed: FixedDecompositionStateV1 =
            read_json(&store.run_dir(&run.id).join(FIXED_DECOMPOSITION_STATE_PATH));
        assert_eq!(fixed.run_kind, WorkflowRunKind::FixedDecompositionV1);
        assert_eq!(fixed.identity.template_version, "fixed-decomposition-v1");
        assert!(!fixed.identity.starting_binary_revision.is_empty());
        assert_eq!(
            fixed.identity.script_digest,
            archon_workflow::workflow_scaffold_hash(FIXED_SCRIPT_SOURCE)
        );
        assert_eq!(
            fixed.identity.project_root_identity,
            path_text(&self.project_root)
        );

        let catalog: CommandCapabilityCatalog =
            read_json(&store.run_dir(&run.id).join(FIXED_CATALOG_PATH));
        assert_eq!(fixed.identity.catalog_digest, catalog.digest);
        assert_eq!(
            catalog.starting_binary_revision,
            fixed.identity.starting_binary_revision
        );
        assert_eq!(catalog.capabilities.len(), 7);

        let args: serde_json::Value = read_json(&store.run_dir(&run.id).join(FIXED_ARGUMENTS_PATH));
        assert_eq!(args["projectRoot"], path_text(&self.project_root));
        assert_eq!(
            args["repositoryRoot"],
            path_text(&self.project_root),
            "the launch grounds the script in the resolved repository"
        );
        assert_eq!(
            run.spec.target_repository_root.as_deref(),
            Some(path_text(&self.project_root).as_str()),
            "the spec names the repository every agent is grounded in"
        );
        let record = archon_workflow::repository_record::read_repository_record(
            &self.project_root.join("tasks/PRD-X"),
        )
        .unwrap()
        .expect("repository.lock is written before provider construction");
        assert_eq!(record.repository_root, path_text(&self.project_root));
        assert_eq!(record.base_commit, archon_workflow::repository_record::UNBORN_BASE_COMMIT);
        assert_eq!(record.decomposition_run_id, run.id);
        assert_eq!(
            args["prdPath"],
            path_text(&self.project_root.join("prds/PRD-X.md"))
        );
        assert_eq!(
            args["taskRoot"],
            path_text(&self.project_root.join("tasks/PRD-X"))
        );
        assert_eq!(
            args["frozenChain"],
            serde_json::json!({"acceptance": false, "skeleton": false, "subjects": [], "bodies": []}),
            "an empty task root freezes nothing"
        );

        let recorded = std::fs::read_to_string(archon_workflow::bundle::record_path(
            &store.run_dir(&run.id),
        ))
        .unwrap();
        assert_eq!(recorded, FIXED_SCRIPT_SOURCE);
        WorkflowBundle::verify(&store, &run.id)?;

        let generated: serde_json::Value =
            read_json(&store.run_dir(&run.id).join("v2/generated-metadata.json"));
        assert_eq!(generated["run_kind"], "fixed_decomposition_v1");
        assert!(
            generated.get("observer_snapshot").is_none(),
            "fixed decomposition must never persist observer intent: {generated:#}"
        );
        assert_eq!(
            generated["scaffold_hash"],
            archon_workflow::workflow_scaffold_hash(FIXED_SCRIPT_SOURCE)
        );

        Err(archon_workflow::WorkflowError::port(
            "barrier observed; stop before execution".to_string(),
        ))
    }
}

pub(super) struct OrderingBarrierFactory {
    pub(super) started_delivered: Arc<AtomicBool>,
}

#[async_trait::async_trait(?Send)]
impl WorkflowLlmClientFactory for OrderingBarrierFactory {
    async fn build_client(
        &self,
        _request: WorkflowLlmClientRequest,
    ) -> archon_workflow::WorkflowResult<Arc<dyn WorkflowLlmClient>> {
        assert!(
            self.started_delivered.load(Ordering::SeqCst),
            "persisted run id must reach the UI before provider construction"
        );
        Err(archon_workflow::WorkflowError::port(
            "ordered barrier observed".to_string(),
        ))
    }
}

pub(super) struct StartedSink {
    pub(super) delivered: Arc<AtomicBool>,
}

#[async_trait::async_trait]
impl archon_workflow::WorkflowUiSink for StartedSink {
    async fn emit(
        &self,
        event: archon_workflow::WorkflowUiEvent,
    ) -> archon_workflow::WorkflowUiResult {
        if let archon_workflow::WorkflowUiEvent::Text(text) = event
            && text.starts_with("Fixed decomposition started: wf-")
        {
            self.delivered.store(true, Ordering::SeqCst);
        }
        Ok(())
    }
}

pub(super) struct ReadyFactory;
pub(super) struct FailingReadyLlm;

#[async_trait::async_trait]
impl WorkflowLlmClient for FailingReadyLlm {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> archon_workflow::WorkflowResult<archon_workflow::WorkflowAgentOutcome> {
        Err(archon_workflow::WorkflowError::port("ready provider stop"))
    }
}

#[async_trait::async_trait(?Send)]
impl WorkflowLlmClientFactory for ReadyFactory {
    async fn build_client(
        &self,
        _request: WorkflowLlmClientRequest,
    ) -> archon_workflow::WorkflowResult<Arc<dyn WorkflowLlmClient>> {
        Ok(Arc::new(FailingReadyLlm))
    }
}

pub(super) struct PanicFactory {
    pub(super) builds: AtomicUsize,
}

#[async_trait::async_trait(?Send)]
impl WorkflowLlmClientFactory for PanicFactory {
    async fn build_client(
        &self,
        _request: WorkflowLlmClientRequest,
    ) -> archon_workflow::WorkflowResult<Arc<dyn WorkflowLlmClient>> {
        self.builds.fetch_add(1, Ordering::SeqCst);
        panic!("provider construction must not occur")
    }
}
