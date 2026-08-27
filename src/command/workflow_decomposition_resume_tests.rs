use super::*;

#[tokio::test]
async fn fixed_resume_cancellation_barrier_skips_provider_and_uses_injected_sink() {
    let project = fixture_project();
    let first = BarrierFactory {
        project_root: project.path().canonicalize().unwrap(),
        builds: AtomicUsize::new(0),
    };
    let _ = run_fixed_decomposition_with_factory(
        project.path(),
        Path::new("prds/PRD-X.md"),
        Path::new("tasks/PRD-X"),
        true,
        &ArchonConfig::default(),
        &empty_env(),
        &first,
    )
    .await;
    let store = WorkflowStore::project(project.path().canonicalize().unwrap());
    let run = store.list_runs().unwrap().pop().unwrap();
    let cancelled = AtomicBool::new(true);
    let factory = PanicFactory {
        builds: AtomicUsize::new(0),
    };
    let delivered = Arc::new(AtomicBool::new(false));

    let error = resume_fixed_decomposition_with_factory_and_sink(
        project.path(),
        &run.id,
        true,
        &ArchonConfig::default(),
        &empty_env(),
        &factory,
        Arc::new(StartedSink {
            delivered: Arc::clone(&delivered),
        }),
        Some(&cancelled),
    )
    .await
    .unwrap_err();

    assert!(
        error.to_string().contains("before provider construction"),
        "{error:#}"
    );
    assert_eq!(factory.builds.load(Ordering::SeqCst), 0);
    assert_eq!(
        store.load_state(&run.id).unwrap().status,
        RunStatus::Cancelled
    );
    assert!(!delivered.load(Ordering::SeqCst));
}

#[tokio::test]
async fn fixed_decomposition_resume_refuses_identity_mismatch_before_provider() {
    let project = fixture_project();
    let barrier = BarrierFactory {
        project_root: project.path().canonicalize().unwrap(),
        builds: AtomicUsize::new(0),
    };
    let _ = run_fixed_decomposition_with_factory(
        project.path(),
        Path::new("prds/PRD-X.md"),
        Path::new("tasks/PRD-X"),
        true,
        &ArchonConfig::default(),
        &empty_env(),
        &barrier,
    )
    .await;
    let store = WorkflowStore::project(project.path().canonicalize().unwrap());
    let run = store.list_runs().unwrap().pop().unwrap();
    let path = store.run_dir(&run.id).join(FIXED_DECOMPOSITION_STATE_PATH);
    let mut state: FixedDecompositionStateV1 = read_json(&path);
    state.identity.script_digest = "different-script".into();
    store
        .write_run_json(&run.id, FIXED_DECOMPOSITION_STATE_PATH, &state)
        .unwrap();
    let factory = PanicFactory {
        builds: AtomicUsize::new(0),
    };

    let error = resume_fixed_decomposition_with_factory(
        project.path(),
        &run.id,
        true,
        &ArchonConfig::default(),
        &empty_env(),
        &factory,
    )
    .await
    .unwrap_err();

    assert!(error.to_string().contains("script_digest"), "{error:#}");
    assert!(error.to_string().contains("do not deploy"), "{error:#}");
    assert_eq!(factory.builds.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn valid_fixed_resume_reuses_existing_run_before_provider_build() {
    let project = fixture_project();
    let first = BarrierFactory {
        project_root: project.path().canonicalize().unwrap(),
        builds: AtomicUsize::new(0),
    };
    let _ = run_fixed_decomposition_with_factory(
        project.path(),
        Path::new("prds/PRD-X.md"),
        Path::new("tasks/PRD-X"),
        true,
        &ArchonConfig::default(),
        &empty_env(),
        &first,
    )
    .await;
    let store = WorkflowStore::project(project.path().canonicalize().unwrap());
    let run = store.list_runs().unwrap().pop().unwrap();
    let resume = BarrierFactory {
        project_root: project.path().canonicalize().unwrap(),
        builds: AtomicUsize::new(0),
    };

    let error = resume_fixed_decomposition_with_factory(
        project.path(),
        &run.id,
        true,
        &ArchonConfig::default(),
        &empty_env(),
        &resume,
    )
    .await
    .unwrap_err();

    assert!(format!("{error:#}").contains("barrier observed"));
    assert_eq!(resume.builds.load(Ordering::SeqCst), 1);
    assert_eq!(store.list_runs().unwrap().len(), 1);
}

#[tokio::test]
async fn second_active_decomposition_for_same_task_root_is_refused() {
    let project = fixture_project();
    let first = BarrierFactory {
        project_root: project.path().canonicalize().unwrap(),
        builds: AtomicUsize::new(0),
    };
    let _ = run_fixed_decomposition_with_factory(
        project.path(),
        Path::new("prds/PRD-X.md"),
        Path::new("tasks/PRD-X"),
        true,
        &ArchonConfig::default(),
        &empty_env(),
        &first,
    )
    .await;
    let second = PanicFactory {
        builds: AtomicUsize::new(0),
    };

    let error = run_fixed_decomposition_with_factory(
        project.path(),
        Path::new("prds/PRD-X.md"),
        Path::new("tasks/PRD-X"),
        true,
        &ArchonConfig::default(),
        &empty_env(),
        &second,
    )
    .await
    .unwrap_err();

    assert!(
        error.to_string().contains("active fixed decomposition"),
        "{error:#}"
    );
    assert_eq!(second.builds.load(Ordering::SeqCst), 0);
    assert_eq!(
        WorkflowStore::project(project.path().canonicalize().unwrap())
            .list_runs()
            .unwrap()
            .len(),
        1
    );
}
