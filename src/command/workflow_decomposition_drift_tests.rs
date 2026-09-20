//! Issue-59: a resume on an upgraded binary records the revision drift instead
//! of refusing, while a changed replay key is still refused.

use super::*;

/// Runs one launch with this binary and pauses it, returning the store, the
/// run id and the log that the launch wrote.
async fn launch_and_pause(project: &Path) -> (WorkflowStore, String, String) {
    let first = BarrierFactory::launch(project.canonicalize().unwrap());
    let _ = run_fixed_decomposition_with_factory(
        project,
        Path::new("prds/PRD-X.md"),
        Path::new("tasks/PRD-X"),
        None,
        true,
        &launch_config(project),
        &empty_env(),
        &first,
    )
    .await;
    let store = WorkflowStore::project(project.canonicalize().unwrap());
    let run = store.list_runs().unwrap().pop().unwrap();
    archon_workflow::LifecycleController::new(store.clone())
        .apply(&run.id, archon_workflow::LifecycleAction::Pause)
        .unwrap();
    let state: FixedDecompositionStateV1 =
        read_json(&store.run_dir(&run.id).join(FIXED_DECOMPOSITION_STATE_PATH));
    let log = std::fs::read_to_string(&state.log_path).unwrap();
    (store, run.id, log)
}

#[tokio::test]
async fn fixed_resume_on_upgraded_binary_proceeds_and_records_the_drift() {
    let project = fixture_project();
    let (store, run_id, launch_log) = launch_and_pause(project.path()).await;
    let persisted: FixedDecompositionStateV1 =
        read_json(&store.run_dir(&run_id).join(FIXED_DECOMPOSITION_STATE_PATH));
    let launched_by = persisted.identity.starting_binary_revision.clone();
    let resume = BarrierFactory::resume(project.path().canonicalize().unwrap());
    let printed = Arc::new(Mutex::new(Vec::new()));

    let error = resume_fixed_decomposition_at_binary_revision(
        project.path(),
        &run_id,
        true,
        &launch_config(project.path()),
        &empty_env(),
        &resume,
        Arc::new(CapturingSink {
            lines: Arc::clone(&printed),
        }),
        None,
        None,
        "upgraded-rev",
    )
    .await
    .unwrap_err();

    // The identity check no longer refuses: the resume reached the provider
    // barrier with the run still paused, exactly as a same-binary resume does.
    assert!(
        format!("{error:#}").contains("barrier observed"),
        "{error:#}"
    );
    assert_eq!(resume.builds.load(Ordering::SeqCst), 1);

    let events = std::fs::read_to_string(store.events_path(&run_id)).unwrap();
    let drift = events
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .find(|event| event["kind"] == "binary_revision_drift")
        .unwrap_or_else(|| panic!("no binary_revision_drift event in {events}"));
    assert_eq!(drift["detail"]["persisted"], launched_by);
    assert_eq!(drift["detail"]["current"], "upgraded-rev");

    let log = std::fs::read_to_string(&persisted.log_path).unwrap();
    assert!(log.starts_with(&launch_log), "{log}");
    assert!(
        log.contains(&format!(
            "transition=binary_revision_drift persisted={launched_by} current=upgraded-rev"
        )),
        "{log}"
    );
    let printed = printed.lock().unwrap();
    assert!(
        printed.iter().any(|line| line
            == &format!("Binary revision drift: persisted={launched_by} current=upgraded-rev\n")),
        "{printed:?}"
    );

    // The persisted identity is the launch record and stays untouched.
    let after: FixedDecompositionStateV1 =
        read_json(&store.run_dir(&run_id).join(FIXED_DECOMPOSITION_STATE_PATH));
    assert_eq!(after.identity, persisted.identity);
}

#[tokio::test]
async fn fixed_resume_on_same_binary_records_no_drift() {
    let project = fixture_project();
    let (store, run_id, _) = launch_and_pause(project.path()).await;
    let resume = BarrierFactory::resume(project.path().canonicalize().unwrap());

    let error = resume_fixed_decomposition_with_factory(
        project.path(),
        &run_id,
        true,
        &launch_config(project.path()),
        &empty_env(),
        &resume,
    )
    .await
    .unwrap_err();

    assert!(
        format!("{error:#}").contains("barrier observed"),
        "{error:#}"
    );
    let events = std::fs::read_to_string(store.events_path(&run_id)).unwrap();
    assert!(!events.contains("binary_revision_drift"), "{events}");
}

#[tokio::test]
async fn fixed_resume_on_upgraded_binary_still_refuses_changed_script_digest() {
    let project = fixture_project();
    let (store, run_id, _) = launch_and_pause(project.path()).await;
    let path = store.run_dir(&run_id).join(FIXED_DECOMPOSITION_STATE_PATH);
    let mut state: FixedDecompositionStateV1 = read_json(&path);
    state.identity.script_digest = "different-script".into();
    store
        .write_run_json(&run_id, FIXED_DECOMPOSITION_STATE_PATH, &state)
        .unwrap();
    let factory = PanicFactory {
        builds: AtomicUsize::new(0),
    };

    let error = resume_fixed_decomposition_at_binary_revision(
        project.path(),
        &run_id,
        true,
        &launch_config(project.path()),
        &empty_env(),
        &factory,
        Arc::new(StartedSink {
            delivered: Arc::new(AtomicBool::new(false)),
        }),
        None,
        None,
        "upgraded-rev",
    )
    .await
    .unwrap_err();

    assert!(error.to_string().contains("script_digest"), "{error:#}");
    assert!(error.to_string().contains("do not deploy"), "{error:#}");
    assert_eq!(factory.builds.load(Ordering::SeqCst), 0);
    let events = std::fs::read_to_string(store.events_path(&run_id)).unwrap();
    assert!(!events.contains("binary_revision_drift"), "{events}");
}

struct CapturingSink {
    lines: Arc<Mutex<Vec<String>>>,
}

#[async_trait::async_trait]
impl archon_workflow::WorkflowUiSink for CapturingSink {
    async fn emit(
        &self,
        event: archon_workflow::WorkflowUiEvent,
    ) -> archon_workflow::WorkflowUiResult {
        if let archon_workflow::WorkflowUiEvent::Text(text) = event {
            self.lines.lock().unwrap().push(text);
        }
        Ok(())
    }
}
