use super::*;

fn pause_run(store: &WorkflowStore, run_id: &str) {
    archon_workflow::LifecycleController::new(store.clone())
        .apply(run_id, archon_workflow::LifecycleAction::Pause)
        .unwrap();
}

#[tokio::test]
async fn fixed_resume_cancellation_barrier_skips_provider_and_uses_injected_sink() {
    let project = fixture_project();
    let first = BarrierFactory::launch(project.path().canonicalize().unwrap());
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
    let store = WorkflowStore::project(project.path().canonicalize().unwrap());
    let run = store.list_runs().unwrap().pop().unwrap();
    pause_run(&store, &run.id);
    let cancelled = AtomicBool::new(true);
    let factory = PanicFactory {
        builds: AtomicUsize::new(0),
    };
    let delivered = Arc::new(AtomicBool::new(false));

    let error = resume_fixed_decomposition_with_factory_and_sink(
        project.path(),
        &run.id,
        true,
        &launch_config(project.path()),
        &empty_env(),
        &factory,
        Arc::new(StartedSink {
            delivered: Arc::clone(&delivered),
        }),
        None,
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
    let barrier = BarrierFactory::launch(project.path().canonicalize().unwrap());
    let _ = run_fixed_decomposition_with_factory(
        project.path(),
        Path::new("prds/PRD-X.md"),
        Path::new("tasks/PRD-X"),
        None,
        true,
        &launch_config(project.path()),
        &empty_env(),
        &barrier,
    )
    .await;
    let store = WorkflowStore::project(project.path().canonicalize().unwrap());
    let run = store.list_runs().unwrap().pop().unwrap();
    pause_run(&store, &run.id);
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
        &launch_config(project.path()),
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
    let first = BarrierFactory::launch(project.path().canonicalize().unwrap());
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
    let store = WorkflowStore::project(project.path().canonicalize().unwrap());
    let run = store.list_runs().unwrap().pop().unwrap();
    pause_run(&store, &run.id);
    let resume = BarrierFactory::resume(project.path().canonicalize().unwrap());

    let error = resume_fixed_decomposition_with_factory(
        project.path(),
        &run.id,
        true,
        &launch_config(project.path()),
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
async fn cancelled_resumable_run_retains_task_root_ownership() {
    let project = fixture_project();
    let first = BarrierFactory::launch(project.path().canonicalize().unwrap());
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
    let store = WorkflowStore::project(project.path().canonicalize().unwrap());
    let run = store.list_runs().unwrap().pop().unwrap();
    assert_eq!(run.status, RunStatus::Cancelled);
    let second = PanicFactory {
        builds: AtomicUsize::new(0),
    };

    let error = run_fixed_decomposition_with_factory(
        project.path(),
        Path::new("prds/PRD-X.md"),
        Path::new("tasks/PRD-X"),
        None,
        true,
        &launch_config(project.path()),
        &empty_env(),
        &second,
    )
    .await
    .unwrap_err();

    assert!(
        error.to_string().contains("already owns task root"),
        "{error:#}"
    );
    assert_eq!(second.builds.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn second_active_decomposition_for_same_task_root_is_refused() {
    let project = fixture_project();
    let first = BarrierFactory::launch(project.path().canonicalize().unwrap());
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
    let store = WorkflowStore::project(project.path().canonicalize().unwrap());
    let mut active = store.list_runs().unwrap().pop().unwrap();
    active.status = RunStatus::Running;
    store.save_state(&active).unwrap();
    let second = PanicFactory {
        builds: AtomicUsize::new(0),
    };

    let error = run_fixed_decomposition_with_factory(
        project.path(),
        Path::new("prds/PRD-X.md"),
        Path::new("tasks/PRD-X"),
        None,
        true,
        &launch_config(project.path()),
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

#[tokio::test]
async fn fixed_resume_appends_marker_and_canonical_resumed_event_before_provider() {
    let project = fixture_project();
    let first = BarrierFactory::launch(project.path().canonicalize().unwrap());
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
    let store = WorkflowStore::project(project.path().canonicalize().unwrap());
    let run = store.list_runs().unwrap().pop().unwrap();
    let state: FixedDecompositionStateV1 =
        read_json(&store.run_dir(&run.id).join(FIXED_DECOMPOSITION_STATE_PATH));
    let initial_log = std::fs::read_to_string(&state.log_path).unwrap();
    pause_run(&store, &run.id);
    let resume = ReadyFactory;

    let _ = resume_fixed_decomposition_with_factory(
        project.path(),
        &run.id,
        true,
        &launch_config(project.path()),
        &empty_env(),
        &resume,
    )
    .await;

    let log = std::fs::read_to_string(&state.log_path).unwrap();
    assert!(log.starts_with(&initial_log));
    assert_eq!(
        log.lines()
            .flat_map(|line| line.split_whitespace())
            .filter(|field| *field == "event=resume")
            .count(),
        1,
        "{log}"
    );
    let events = std::fs::read_to_string(store.events_path(&run.id)).unwrap();
    assert!(events.contains("\"kind\":\"resumed\""), "{events}");
}

#[cfg(unix)]
#[tokio::test]
async fn fixed_resume_event_failure_restores_admitted_paused_state() {
    use std::os::unix::fs::PermissionsExt;

    let project = fixture_project();
    let first = BarrierFactory::launch(project.path().canonicalize().unwrap());
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
    let store = WorkflowStore::project(project.path().canonicalize().unwrap());
    let run = store.list_runs().unwrap().pop().unwrap();
    pause_run(&store, &run.id);
    let before = store.load_state(&run.id).unwrap();
    let events = store.events_path(&run.id);
    std::fs::set_permissions(&events, std::fs::Permissions::from_mode(0o400)).unwrap();

    let error = resume_fixed_decomposition_with_factory(
        project.path(),
        &run.id,
        true,
        &launch_config(project.path()),
        &empty_env(),
        &ReadyFactory,
    )
    .await
    .unwrap_err();
    std::fs::set_permissions(&events, std::fs::Permissions::from_mode(0o600)).unwrap();

    let after = store.load_state(&run.id).unwrap();
    assert!(error.to_string().contains("resume lifecycle"), "{error:#}");
    assert_eq!(after.status, RunStatus::Paused);
    assert_eq!(after.generation, before.generation);
}

#[tokio::test]
async fn fixed_resume_log_failure_preserves_paused_state_before_transition() {
    let project = fixture_project();
    let first = BarrierFactory::launch(project.path().canonicalize().unwrap());
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
    let store = WorkflowStore::project(project.path().canonicalize().unwrap());
    let run = store.list_runs().unwrap().pop().unwrap();
    pause_run(&store, &run.id);
    let state: FixedDecompositionStateV1 =
        read_json(&store.run_dir(&run.id).join(FIXED_DECOMPOSITION_STATE_PATH));
    std::fs::remove_file(&state.log_path).unwrap();
    std::fs::create_dir(&state.log_path).unwrap();

    let error = resume_fixed_decomposition_with_factory(
        project.path(),
        &run.id,
        true,
        &launch_config(project.path()),
        &empty_env(),
        &ReadyFactory,
    )
    .await
    .unwrap_err();

    assert!(error.to_string().contains("non-symlink file"), "{error:#}");
    assert_eq!(store.load_state(&run.id).unwrap().status, RunStatus::Paused);
}

#[tokio::test]
async fn fixed_resume_rejects_nonresumable_status_without_mutation_or_provider() {
    let project = fixture_project();
    let first = BarrierFactory::launch(project.path().canonicalize().unwrap());
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
    let store = WorkflowStore::project(project.path().canonicalize().unwrap());
    let mut run = store.list_runs().unwrap().pop().unwrap();
    run.status = RunStatus::Planned;
    store.save_state(&run).unwrap();
    let factory = PanicFactory {
        builds: AtomicUsize::new(0),
    };

    let error = resume_fixed_decomposition_with_factory(
        project.path(),
        &run.id,
        true,
        &launch_config(project.path()),
        &empty_env(),
        &factory,
    )
    .await
    .unwrap_err();

    assert!(
        error.to_string().contains("paused or cancelled"),
        "{error:#}"
    );
    assert_eq!(factory.builds.load(Ordering::SeqCst), 0);
    assert_eq!(
        store.load_state(&run.id).unwrap().status,
        RunStatus::Planned
    );
}

#[tokio::test]
async fn fixed_resume_preparation_failure_leaves_paused_state_and_no_resume_evidence() {
    let project = fixture_project();
    let first = BarrierFactory::launch(project.path().canonicalize().unwrap());
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
    let store = WorkflowStore::project(project.path().canonicalize().unwrap());
    let run = store.list_runs().unwrap().pop().unwrap();
    pause_run(&store, &run.id);
    std::fs::remove_file(store.run_dir(&run.id).join(FIXED_ARGUMENTS_PATH)).unwrap();
    let state: FixedDecompositionStateV1 =
        read_json(&store.run_dir(&run.id).join(FIXED_DECOMPOSITION_STATE_PATH));
    let before_log = std::fs::read_to_string(&state.log_path).unwrap();
    let factory = PanicFactory {
        builds: AtomicUsize::new(0),
    };

    let _ = resume_fixed_decomposition_with_factory(
        project.path(),
        &run.id,
        true,
        &launch_config(project.path()),
        &empty_env(),
        &factory,
    )
    .await
    .unwrap_err();

    assert_eq!(store.load_state(&run.id).unwrap().status, RunStatus::Paused);
    assert_eq!(factory.builds.load(Ordering::SeqCst), 0);
    assert_eq!(
        std::fs::read_to_string(&state.log_path).unwrap(),
        before_log
    );
    let events = std::fs::read_to_string(store.events_path(&run.id)).unwrap();
    assert!(!events.contains("\"kind\":\"resumed\""), "{events}");
}
