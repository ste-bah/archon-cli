//! A fixed run launched by a binary built before the Issue 349 check-policy
//! binding has no `check_environment_policy` key, and its launch digest
//! anchor covers only identity, arguments, catalog and route. Such a run must
//! still resume; a run whose metadata and anchor disagree must not.

use super::*;
use crate::command::workflow_decompose::FIXED_LAUNCH_DIGEST_PERMISSION;
use crate::command::workflow_provider_route::TrustedProviderRouteSnapshot;

const METADATA: &str = "v2/generated-metadata.json";

/// Rewrite a paused fixed run into the form an older binary launched.
fn to_pre_binding(store: &WorkflowStore, run_id: &str) {
    let dir = store.run_dir(run_id);
    let mut metadata: serde_json::Value = read_json(&dir.join(METADATA));
    metadata
        .as_object_mut()
        .unwrap()
        .remove("check_environment_policy");
    store.write_run_json(run_id, METADATA, &metadata).unwrap();
    let state: FixedDecompositionStateV1 = read_json(&dir.join(FIXED_DECOMPOSITION_STATE_PATH));
    let arguments: serde_json::Value = read_json(&dir.join(FIXED_ARGUMENTS_PATH));
    let catalog: CommandCapabilityCatalog = read_json(&dir.join(FIXED_CATALOG_PATH));
    let route: TrustedProviderRouteSnapshot =
        read_json(&dir.join("decomposition/provider-route.json"));
    let bytes = serde_json::to_vec(&(&state.identity, &arguments, &catalog, &route)).unwrap();
    let anchor = archon_workflow::task_set_contract::content_digest(&bytes);
    let mut run = store.load_state(run_id).unwrap();
    run.spec.permissions.insert(
        FIXED_LAUNCH_DIGEST_PERMISSION.to_string(),
        serde_json::Value::String(anchor),
    );
    store.save_state(&run).unwrap();
    WorkflowBundle::create_for_run(
        store,
        &run,
        FIXED_SCRIPT_SOURCE,
        archon_workflow::WorkflowBundleOrigin::GeneratedHarness,
    )
    .unwrap();
}

async fn paused_run(project: &Path, config: &ArchonConfig) -> (WorkflowStore, String) {
    let launch = BarrierFactory::launch(project.canonicalize().map(plain).unwrap());
    let _ = run_fixed_decomposition_with_factory(
        project,
        Path::new("prds/PRD-X.md"),
        Path::new("tasks/PRD-X"),
        None,
        true,
        config,
        &empty_env(),
        &launch,
    )
    .await;
    let store = WorkflowStore::project(project.canonicalize().map(plain).unwrap());
    let run = store.list_runs().unwrap().pop().unwrap();
    pause_run(&store, &run.id);
    (store, run.id)
}

#[tokio::test]
async fn pre_binding_fixed_run_resumes_past_admission() {
    let project = fixture_project();
    let config = launch_config(project.path());
    let (store, run_id) = paused_run(project.path(), &config).await;
    to_pre_binding(&store, &run_id);
    let resume = BarrierFactory::resume(project.path().canonicalize().map(plain).unwrap());

    let error = resume_fixed_decomposition_with_factory(
        project.path(),
        &run_id,
        true,
        &config,
        &empty_env(),
        &resume,
    )
    .await
    .unwrap_err();

    assert!(
        format!("{error:#}").contains("barrier observed"),
        "{error:#}"
    );
    assert_eq!(resume.builds.load(Ordering::SeqCst), 1);
    let metadata: serde_json::Value = read_json(&store.run_dir(&run_id).join(METADATA));
    assert!(metadata.get("check_environment_policy").is_none());
}

#[tokio::test]
async fn pre_binding_anchor_with_a_recorded_binding_is_refused() {
    for binding in [
        serde_json::Value::Null,
        serde_json::json!({"toolchain_path": "/usr/bin:/bin", "bound": {}, "forwarded": []}),
    ] {
        let project = fixture_project();
        let config = launch_config(project.path());
        let (store, run_id) = paused_run(project.path(), &config).await;
        to_pre_binding(&store, &run_id);
        let path = store.run_dir(&run_id).join(METADATA);
        let mut metadata: serde_json::Value = read_json(&path);
        metadata["check_environment_policy"] = binding;
        store.write_run_json(&run_id, METADATA, &metadata).unwrap();
        let factory = PanicFactory {
            builds: AtomicUsize::new(0),
        };

        let error = resume_fixed_decomposition_with_factory(
            project.path(),
            &run_id,
            true,
            &config,
            &empty_env(),
            &factory,
        )
        .await
        .unwrap_err();

        assert!(
            error.to_string().contains("launch snapshot differs"),
            "{error:#}"
        );
        assert_eq!(factory.builds.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn current_anchor_without_a_binding_names_the_removed_binding() {
    let project = fixture_project();
    let config = launch_config(project.path());
    let (store, run_id) = paused_run(project.path(), &config).await;
    let path = store.run_dir(&run_id).join(METADATA);
    let mut metadata: serde_json::Value = read_json(&path);
    metadata
        .as_object_mut()
        .unwrap()
        .remove("check_environment_policy");
    store.write_run_json(&run_id, METADATA, &metadata).unwrap();
    let factory = PanicFactory {
        builds: AtomicUsize::new(0),
    };

    let error = resume_fixed_decomposition_with_factory(
        project.path(),
        &run_id,
        true,
        &config,
        &empty_env(),
        &factory,
    )
    .await
    .unwrap_err();

    assert!(
        error.to_string().contains("removed after launch"),
        "{error:#}"
    );
    assert_eq!(factory.builds.load(Ordering::SeqCst), 0);
}
