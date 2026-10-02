//! The task root a launch names (#228): a destination that does not exist yet
//! is created under its existing parent, and a launch that fails before any
//! work starts gives its task root back.

use super::*;

/// A provider whose client never builds, as a launch with no usable
/// credentials sees. It counts the attempts, so a test can tell a launch that
/// reached provider construction from one refused before it.
struct UnbuildableFactory {
    builds: AtomicUsize,
}

impl UnbuildableFactory {
    fn new() -> Self {
        Self {
            builds: AtomicUsize::new(0),
        }
    }

    fn builds(&self) -> usize {
        self.builds.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait(?Send)]
impl WorkflowLlmClientFactory for UnbuildableFactory {
    async fn build_client(
        &self,
        _request: WorkflowLlmClientRequest,
    ) -> archon_workflow::WorkflowResult<Arc<dyn WorkflowLlmClient>> {
        self.builds.fetch_add(1, Ordering::SeqCst);
        Err(archon_workflow::WorkflowError::port(
            "provider client unavailable".to_string(),
        ))
    }
}

async fn launch(
    project: &Path,
    tasks: &str,
    factory: &dyn WorkflowLlmClientFactory,
) -> anyhow::Result<String> {
    run_fixed_decomposition_with_factory(
        project,
        Path::new("prds/PRD-X.md"),
        Path::new(tasks),
        None,
        true,
        &launch_config(project),
        &empty_env(),
        factory,
    )
    .await
}

fn store_of(project: &tempfile::TempDir) -> WorkflowStore {
    WorkflowStore::project(
        project
            .path()
            .canonicalize()
            .map(archon_shell::paths::plain)
            .unwrap(),
    )
}

fn only_run(store: &WorkflowStore) -> archon_workflow::WorkflowRun {
    let mut runs = store.list_runs().unwrap();
    assert_eq!(runs.len(), 1, "{runs:?}");
    runs.pop().unwrap()
}

#[tokio::test]
async fn a_missing_task_root_is_created_under_its_existing_parent() {
    let project = fixture_project();
    let tasks = project.path().join("tasks/PRD-NEW");
    assert!(!tasks.exists());
    let factory = UnbuildableFactory::new();

    let error = launch(project.path(), "tasks/PRD-NEW", &factory)
        .await
        .unwrap_err();

    assert!(
        format!("{error:#}").contains("provider client unavailable"),
        "{error:#}"
    );
    assert_eq!(
        factory.builds(),
        1,
        "the launch reached provider construction"
    );
    assert!(tasks.is_dir());
    let store = store_of(&project);
    let run = only_run(&store);
    let state: FixedDecompositionStateV1 =
        read_json(&store.run_dir(&run.id).join(FIXED_DECOMPOSITION_STATE_PATH));
    assert_eq!(
        state.identity.task_root_identity,
        path_text(
            &tasks
                .canonicalize()
                .map(archon_shell::paths::plain)
                .unwrap()
        )
    );
}

#[tokio::test]
async fn an_existing_empty_task_root_is_accepted() {
    let project = fixture_project();
    let factory = UnbuildableFactory::new();

    let error = launch(project.path(), "tasks/PRD-X", &factory)
        .await
        .unwrap_err();

    assert!(
        format!("{error:#}").contains("provider client unavailable"),
        "{error:#}"
    );
    assert_eq!(factory.builds(), 1);
}

#[tokio::test]
async fn a_missing_parent_is_a_clear_error_and_creates_nothing() {
    let project = fixture_project();
    let factory = UnbuildableFactory::new();

    let error = launch(project.path(), "absent/PRD-NEW", &factory)
        .await
        .unwrap_err();

    let text = format!("{error:#}");
    assert!(text.contains("parent directory"), "{text}");
    assert!(text.contains("absent"), "{text}");
    assert_eq!(factory.builds(), 0);
    assert!(!project.path().join("absent").exists());
    assert!(store_of(&project).list_runs().unwrap().is_empty());
}

#[tokio::test]
async fn a_missing_task_root_outside_the_project_is_refused_and_not_created() {
    let outer = tempfile::tempdir().unwrap();
    let project = fixture_project();
    let factory = UnbuildableFactory::new();
    let outside = outer.path().join("escaped-tasks");
    let tasks = outside.to_string_lossy().into_owned();

    let error = launch(project.path(), &tasks, &factory).await.unwrap_err();

    assert!(
        format!("{error:#}").contains("escapes project root"),
        "{error:#}"
    );
    assert_eq!(factory.builds(), 0);
    assert!(!outside.exists());
}

#[tokio::test]
async fn a_launch_refused_before_its_claim_does_not_create_the_task_root() {
    let project = fixture_project();
    let factory = UnbuildableFactory::new();

    let error = run_fixed_decomposition_with_factory(
        project.path(),
        Path::new("prds/PRD-X.md"),
        Path::new("tasks/PRD-NEW"),
        None,
        true,
        &ArchonConfig::default(),
        &empty_env(),
        &factory,
    )
    .await
    .unwrap_err();

    assert!(error.to_string().contains("--repository"), "{error:#}");
    assert!(!project.path().join("tasks/PRD-NEW").exists());
}

#[tokio::test]
async fn a_non_empty_task_root_recording_another_repository_is_refused() {
    let project = fixture_project();
    let other = tempfile::tempdir().unwrap();
    git_init(other.path());
    let first = UnbuildableFactory::new();
    let _ = launch(project.path(), "tasks/PRD-X", &first).await;
    let factory = UnbuildableFactory::new();
    let mut config = launch_config(project.path());
    config.workflow.repository_root = Some(other.path().to_path_buf());

    let error = run_fixed_decomposition_with_factory(
        project.path(),
        Path::new("prds/PRD-X.md"),
        Path::new("tasks/PRD-X"),
        None,
        true,
        &config,
        &empty_env(),
        &factory,
    )
    .await
    .unwrap_err();

    assert!(
        error.to_string().contains("records repository"),
        "{error:#}"
    );
    assert_eq!(factory.builds(), 0);
}

#[tokio::test]
async fn a_task_root_owned_by_another_active_run_is_refused() {
    let project = fixture_project();
    let _ = launch(project.path(), "tasks/PRD-X", &UnbuildableFactory::new()).await;
    let store = store_of(&project);
    let mut active = only_run(&store);
    active.status = RunStatus::Running;
    store.save_state(&active).unwrap();
    let factory = UnbuildableFactory::new();

    let error = launch(project.path(), "tasks/PRD-X", &factory)
        .await
        .unwrap_err();

    assert!(
        error.to_string().contains("already owns task root"),
        "{error:#}"
    );
    assert_eq!(factory.builds(), 0);
}

#[tokio::test]
async fn a_provider_build_failure_leaves_the_task_root_unowned() {
    let project = fixture_project();
    let first = UnbuildableFactory::new();
    let _ = launch(project.path(), "tasks/PRD-X", &first).await;
    let store = store_of(&project);
    let failed = only_run(&store);
    assert_eq!(failed.status, RunStatus::Cancelled);
    let second = UnbuildableFactory::new();

    let error = launch(project.path(), "tasks/PRD-X", &second)
        .await
        .unwrap_err();

    assert!(
        format!("{error:#}").contains("provider client unavailable"),
        "a second decompose must proceed to provider construction: {error:#}"
    );
    assert_eq!(second.builds(), 1);
    assert_eq!(store.list_runs().unwrap().len(), 2);
}

#[tokio::test]
async fn a_released_run_resumes_while_its_task_root_is_free() {
    let project = fixture_project();
    let _ = launch(project.path(), "tasks/PRD-X", &UnbuildableFactory::new()).await;
    let store = store_of(&project);
    let run = only_run(&store);
    let resume = UnbuildableFactory::new();

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

    assert!(
        format!("{error:#}").contains("provider client unavailable"),
        "{error:#}"
    );
    assert_eq!(resume.builds(), 1);
}

#[tokio::test]
async fn a_released_run_cannot_resume_once_another_run_owns_its_task_root() {
    let project = fixture_project();
    let _ = launch(project.path(), "tasks/PRD-X", &UnbuildableFactory::new()).await;
    let store = store_of(&project);
    let released = only_run(&store);
    let _ = launch(project.path(), "tasks/PRD-X", &UnbuildableFactory::new()).await;
    let mut owner = store
        .list_runs()
        .unwrap()
        .into_iter()
        .find(|run| run.id != released.id)
        .expect("the second launch created a run");
    owner.status = RunStatus::Running;
    store.save_state(&owner).unwrap();

    let error = resume_fixed_decomposition_with_factory(
        project.path(),
        &released.id,
        true,
        &launch_config(project.path()),
        &empty_env(),
        &ReadyFactory,
    )
    .await
    .unwrap_err();

    assert!(
        error.to_string().contains("already owns task root"),
        "{error:#}"
    );
    let after = store.load_state(&released.id).unwrap();
    assert_eq!(
        after.status,
        RunStatus::Cancelled,
        "the refused resume changed nothing"
    );
}
