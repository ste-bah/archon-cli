use super::*;

#[tokio::test]
async fn workflow_decompose_resume_event_precedes_any_v2_call_record_on_a_paused_fixed_run() {
    let project = fixture_project();
    let first = BarrierFactory::launch(project.path().canonicalize().map(plain).unwrap());
    let _ = run_fixed_decomposition_with_factory(
        project.path(),
        Path::new("prds/PRD-X.md"),
        Path::new("tasks/PRD-X"),
        None,
        true,
        &launch_config(project.path()),
        &empty_env(),
        &first,
    )
    .await;
    let store = WorkflowStore::project(project.path().canonicalize().map(plain).unwrap());
    let run = store.list_runs().unwrap().pop().unwrap();
    pause_run(&store, &run.id);
    let v2 = archon_workflow::WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    let records_before = v2.load_call_records().unwrap();

    let factory = ResumeOrderingFactory {
        project_root: project.path().canonicalize().map(plain).unwrap(),
        calls: Arc::new(AtomicUsize::new(0)),
    };
    let output = resume_fixed_decomposition_with_factory(
        project.path(),
        &run.id,
        true,
        &launch_config(project.path()),
        &empty_env(),
        &factory,
    )
    .await
    .unwrap();

    assert!(output.contains("Fixed decomposition paused"), "{output}");
    assert_eq!(records_before.len(), 0);
    assert_eq!(factory.calls.load(Ordering::SeqCst), 1);
}

struct ResumeOrderingFactory {
    project_root: PathBuf,
    calls: Arc<AtomicUsize>,
}

#[async_trait::async_trait(?Send)]
impl WorkflowLlmClientFactory for ResumeOrderingFactory {
    async fn build_client(
        &self,
        request: WorkflowLlmClientRequest,
    ) -> archon_workflow::WorkflowResult<Arc<dyn WorkflowLlmClient>> {
        Ok(Arc::new(ResumeOrderingLlm {
            store: WorkflowStore::project(self.project_root.clone()),
            run_id: request.session_id,
            calls: Arc::clone(&self.calls),
        }))
    }
}

struct ResumeOrderingLlm {
    store: WorkflowStore,
    run_id: String,
    calls: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl WorkflowLlmClient for ResumeOrderingLlm {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> archon_workflow::WorkflowResult<archon_workflow::WorkflowAgentOutcome> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let (events, damaged) =
            archon_workflow::events::read_event_log(&self.store.events_path(&self.run_id))?;
        assert_eq!(damaged, 0);
        assert!(
            events
                .iter()
                .any(|event| event.kind == archon_workflow::WorkflowEventKind::Resumed),
            "Resume event must exist before the first provider invocation"
        );
        let v2 = archon_workflow::WorkflowV2ResultStore::new(
            self.store.run_dir(&self.run_id).join("v2"),
        );
        assert_eq!(
            v2.load_call_records()?.len(),
            0,
            "the executor must not create a v2 call record before invoking the provider"
        );
        Err(archon_workflow::WorkflowError::port(
            "ordering barrier observed",
        ))
    }
}
