use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use archon_core::config::{ArchonConfig, GateMode};
use archon_workflow::{
    CommandCapabilityCatalog, FixedDecompositionStateV1, RunStatus, WorkflowBundle,
    WorkflowLlmClient, WorkflowLlmClientFactory, WorkflowLlmClientRequest, WorkflowRunKind,
    WorkflowStore,
};

use super::workflow_decompose::{
    DECOMPOSE_GATE_OFF_REMEDY, FIXED_ARGUMENTS_PATH, FIXED_CATALOG_PATH,
    FIXED_DECOMPOSITION_STATE_PATH, FIXED_SCRIPT_SOURCE, run_fixed_decomposition_with_factory,
};

struct BarrierFactory {
    project_root: PathBuf,
    builds: AtomicUsize,
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
        assert_eq!(run.status, RunStatus::Planned);
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
        assert_eq!(
            generated["scaffold_hash"],
            archon_workflow::workflow_scaffold_hash(FIXED_SCRIPT_SOURCE)
        );

        Err(archon_workflow::WorkflowError::port(
            "barrier observed; stop before execution".to_string(),
        ))
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
async fn fixed_decomposition_run_is_persisted_before_provider_construction() {
    let project = fixture_project();
    let factory = BarrierFactory {
        project_root: project.path().canonicalize().unwrap(),
        builds: AtomicUsize::new(0),
    };

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
        "# PRD X\n\n## Requirements\n\nREQ-X-001: prove the fixture.\n",
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
