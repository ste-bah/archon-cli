use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use archon_core::config::{ArchonConfig, GateMode};
use archon_workflow::{
    CommandCapabilityCatalog, FixedDecompositionStateV1, RunStatus, WorkflowBundle,
    WorkflowLlmClient, WorkflowLlmClientFactory, WorkflowLlmClientRequest, WorkflowRunKind,
    WorkflowStore,
};

use super::workflow_decompose::{
    DECOMPOSE_GATE_OFF_REMEDY, FIXED_ARGUMENTS_PATH, FIXED_CATALOG_PATH,
    FIXED_DECOMPOSITION_STATE_PATH, FIXED_SCRIPT_SOURCE, resume_fixed_decomposition_with_factory,
    resume_fixed_decomposition_with_factory_and_sink, run_fixed_decomposition_with_factory,
};

struct BarrierFactory {
    project_root: PathBuf,
    builds: AtomicUsize,
    expected_status: RunStatus,
}

impl BarrierFactory {
    fn launch(project_root: PathBuf) -> Self {
        Self {
            project_root,
            builds: AtomicUsize::new(0),
            expected_status: RunStatus::Planned,
        }
    }

    fn resume(project_root: PathBuf) -> Self {
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
        assert_eq!(catalog.capabilities.len(), 5);

        let args: serde_json::Value = read_json(&store.run_dir(&run.id).join(FIXED_ARGUMENTS_PATH));
        assert_eq!(args["projectRoot"], path_text(&self.project_root));
        assert_eq!(
            args["prdPath"],
            path_text(&self.project_root.join("prds/PRD-X.md"))
        );
        assert_eq!(
            args["taskRoot"],
            path_text(&self.project_root.join("tasks/PRD-X"))
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

struct OrderingBarrierFactory {
    started_delivered: Arc<AtomicBool>,
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

struct StartedSink {
    delivered: Arc<AtomicBool>,
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

struct ReadyFactory;
struct FailingReadyLlm;

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

struct PanicFactory {
    builds: AtomicUsize,
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

#[tokio::test]
async fn fixed_decomposition_publishes_persisted_run_id_before_provider_construction() {
    let project = fixture_project();
    let delivered = Arc::new(AtomicBool::new(false));
    let factory = OrderingBarrierFactory {
        started_delivered: Arc::clone(&delivered),
    };

    let error = super::workflow_decompose::run_fixed_decomposition_with_factory_and_sink(
        project.path(),
        Path::new("prds/PRD-X.md"),
        Path::new("tasks/PRD-X"),
        true,
        &ArchonConfig::default(),
        &empty_env(),
        &factory,
        Arc::new(StartedSink {
            delivered: Arc::clone(&delivered),
        }),
        None,
        None,
        None,
    )
    .await
    .unwrap_err();

    assert!(format!("{error:#}").contains("ordered barrier observed"));
    assert!(delivered.load(Ordering::SeqCst));
    let store = WorkflowStore::project(project.path().canonicalize().unwrap());
    assert_eq!(store.list_runs().unwrap().len(), 1);
}

#[tokio::test]
async fn fixed_launch_marker_failure_publishes_and_terminalizes_run() {
    let project = fixture_project();
    let log_path = project.path().join("tasks/PRD-X/.decompose.log");
    std::fs::create_dir(&log_path).unwrap();
    let factory = PanicFactory {
        builds: AtomicUsize::new(0),
    };
    let owner_run_id = Mutex::new(None);

    let error = super::workflow_decompose::run_fixed_decomposition_with_factory_and_sink(
        project.path(),
        Path::new("prds/PRD-X.md"),
        Path::new("tasks/PRD-X"),
        true,
        &ArchonConfig::default(),
        &empty_env(),
        &factory,
        Arc::new(StartedSink {
            delivered: Arc::new(AtomicBool::new(false)),
        }),
        Some(&owner_run_id),
        None,
        None,
    )
    .await
    .unwrap_err();

    assert!(error.to_string().contains("not a regular non-symlink file"));
    assert_eq!(factory.builds.load(Ordering::SeqCst), 0);
    let run_id = owner_run_id
        .lock()
        .unwrap()
        .clone()
        .expect("published run id");
    let store = WorkflowStore::project(project.path().canonicalize().unwrap());
    let run = store.load_state(&run_id).unwrap();
    assert_eq!(run.status, RunStatus::Cancelled);
}

#[tokio::test]
async fn cancellation_requested_at_persistence_barrier_skips_provider_construction() {
    let project = fixture_project();
    let factory = PanicFactory {
        builds: AtomicUsize::new(0),
    };
    let cancelled = AtomicBool::new(true);

    let error = super::workflow_decompose::run_fixed_decomposition_with_factory_and_sink(
        project.path(),
        Path::new("prds/PRD-X.md"),
        Path::new("tasks/PRD-X"),
        true,
        &ArchonConfig::default(),
        &empty_env(),
        &factory,
        Arc::new(StartedSink {
            delivered: Arc::new(AtomicBool::new(false)),
        }),
        None,
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
    let store = WorkflowStore::project(project.path().canonicalize().unwrap());
    assert_eq!(store.list_runs().unwrap()[0].status, RunStatus::Cancelled);
}

#[tokio::test]
async fn fixed_decomposition_run_is_persisted_before_provider_construction() {
    let project = fixture_project();
    let factory = BarrierFactory::launch(project.path().canonicalize().unwrap());

    let error = run_fixed_decomposition_with_factory(
        project.path(),
        Path::new("prds/PRD-X.md"),
        Path::new("tasks/PRD-X"),
        true,
        &ArchonConfig::default(),
        &empty_env(),
        &factory,
    )
    .await
    .unwrap_err();

    assert!(
        format!("{error:#}").contains("barrier observed"),
        "{error:#}"
    );
    assert_eq!(factory.builds.load(Ordering::SeqCst), 1);
    let store = WorkflowStore::project(project.path().canonicalize().unwrap());
    assert_eq!(store.list_runs().unwrap()[0].status, RunStatus::Cancelled);
}

#[test]
fn obsolete_launch_cleanup_never_cancels_newer_generation_owner() {
    let project = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(project.path());
    let run = store
        .create_run(archon_workflow::WorkflowSpec {
            schema: archon_workflow::spec::WORKFLOW_SCHEMA.into(),
            name: "launch-cleanup-generation".into(),
            task: "test".into(),
            target_repository_root: None,
            max_parallelism: 1,
            max_agents: 1,
            stages: Vec::new(),
            permissions: Default::default(),
            learning_hooks: Vec::new(),
        })
        .unwrap();
    let lifecycle = archon_workflow::LifecycleController::new(store.clone());
    lifecycle
        .apply(&run.id, archon_workflow::LifecycleAction::Pause)
        .unwrap();
    lifecycle
        .apply(&run.id, archon_workflow::LifecycleAction::Resume)
        .unwrap();

    super::workflow_decompose::cancel_active_launch_failure(&store, &run.id, run.generation)
        .unwrap();

    assert_eq!(
        store.load_state(&run.id).unwrap().status,
        RunStatus::Running
    );
}

#[tokio::test]
async fn decompose_gate_mode_off_refuses_before_paths_run_or_provider() {
    let project = tempfile::tempdir().unwrap();
    let factory = PanicFactory {
        builds: AtomicUsize::new(0),
    };
    let mut config = ArchonConfig::default();
    config.workflow.gate_mode = GateMode::Off;

    let error = run_fixed_decomposition_with_factory(
        project.path(),
        Path::new("missing-prd.md"),
        Path::new("missing-tasks"),
        true,
        &config,
        &empty_env(),
        &factory,
    )
    .await
    .unwrap_err();

    assert_eq!(error.to_string(), DECOMPOSE_GATE_OFF_REMEDY);
    assert_eq!(factory.builds.load(Ordering::SeqCst), 0);
    assert!(!project.path().join(".archon/workflows").exists());
    assert!(!project.path().join("missing-tasks").exists());
}

#[tokio::test]
async fn cli_workflow_decompose_requires_yes_before_run_creation() {
    let project = fixture_project();
    let factory = PanicFactory {
        builds: AtomicUsize::new(0),
    };

    let error = run_fixed_decomposition_with_factory(
        project.path(),
        Path::new("prds/PRD-X.md"),
        Path::new("tasks/PRD-X"),
        false,
        &ArchonConfig::default(),
        &empty_env(),
        &factory,
    )
    .await
    .unwrap_err();

    assert!(error.to_string().contains("requires --yes"), "{error:#}");
    assert_eq!(factory.builds.load(Ordering::SeqCst), 0);
    assert!(!project.path().join(".archon/workflows").exists());
}

fn empty_env() -> archon_core::env_vars::ArchonEnvVars {
    archon_core::env_vars::load_env_vars_from(&std::collections::HashMap::new())
}

fn fixture_project() -> tempfile::TempDir {
    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(project.path().join("prds")).unwrap();
    std::fs::create_dir_all(project.path().join("tasks/PRD-X")).unwrap();
    std::fs::write(
        project.path().join("prds/PRD-X.md"),
        "# PRD X\n\n## Requirements\n\n| ID | Requirement |\n|---|---|\n| REQ-X-001 | Prove the fixture. |\n\n## Acceptance Criteria\n\n| ID | Criterion |\n|---|---|\n| AC-X-001 | The fixture is proven. |\n",
    )
    .unwrap();
    project
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> T {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[path = "workflow_decompose_claim_tests.rs"]
mod workflow_decompose_claim_tests;
#[path = "workflow_decomposition_integrity_tests.rs"]
mod workflow_decomposition_integrity_tests;
#[path = "workflow_decomposition_resume_tests.rs"]
mod workflow_decomposition_resume_tests;

#[test]
fn fixed_decomposition_identity_is_read_only_and_matches_embedded_inputs() {
    let source = include_str!("workflow_decompose_identity.rs");
    assert!(!source.contains("WorkflowStore"));
    assert!(!source.contains("std::fs"));
    let value = super::workflow_decompose_identity::fixed_decomposition_identity().unwrap();
    assert_eq!(value["template_version"], "fixed-decomposition-v1");
    assert_eq!(value["binary_revision"], env!("ARCHON_GIT_HASH"));
    assert_eq!(
        value["script_digest"],
        archon_workflow::workflow_scaffold_hash(FIXED_SCRIPT_SOURCE)
    );
    assert!(value["catalog_digest"].as_str().is_some());
}

#[tokio::test]
async fn fixed_launch_writes_identity_log_header_before_provider_construction() {
    let project = fixture_project();
    let factory = BarrierFactory::launch(project.path().canonicalize().unwrap());
    let _ = run_fixed_decomposition_with_factory(
        project.path(),
        Path::new("prds/PRD-X.md"),
        Path::new("tasks/PRD-X"),
        true,
        &ArchonConfig::default(),
        &empty_env(),
        &factory,
    )
    .await;
    let store = WorkflowStore::project(project.path().canonicalize().unwrap());
    let run = store.list_runs().unwrap().pop().unwrap();
    let state: FixedDecompositionStateV1 =
        read_json(&store.run_dir(&run.id).join(FIXED_DECOMPOSITION_STATE_PATH));
    let log = std::fs::read_to_string(&state.log_path).unwrap();
    let first = log.lines().next().expect("identity log header");
    assert!(first.contains("event=run_started"), "{log}");
    assert!(first.contains(&format!("run_id={}", run.id)), "{log}");
    assert!(
        first.contains(&state.identity.starting_binary_revision),
        "{log}"
    );
    assert!(first.contains(&state.identity.script_digest), "{log}");
    assert!(first.contains(&state.identity.catalog_digest), "{log}");
}
