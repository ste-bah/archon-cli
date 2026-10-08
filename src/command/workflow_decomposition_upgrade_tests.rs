use super::*;
use crate::command::workflow_decompose::{
    FIXED_GENERATED_METADATA_PATH, FIXED_LAUNCH_DIGEST_PERMISSION, FIXED_PROVIDER_ROUTE_PATH,
    fixed_launch_digest,
};
use crate::command::workflow_host_command_catalog::fixed_decomposition_catalog;
use archon_workflow::{WorkflowBundleOrigin, workflow_scaffold_hash};

async fn resume_upgraded(component: &str) {
    let project = fixture_project();
    let (store, run_id, _) =
        super::workflow_decomposition_drift_tests::launch_and_pause(project.path()).await;
    let mut state: FixedDecompositionStateV1 =
        read_json(&store.run_dir(&run_id).join(FIXED_DECOMPOSITION_STATE_PATH));
    let source = if component == "script" || component == "all" {
        format!("{FIXED_SCRIPT_SOURCE}\n// earlier harness\n")
    } else {
        FIXED_SCRIPT_SOURCE.to_string()
    };
    state.identity.script_digest = archon_workflow::workflow_scaffold_hash(&source);
    if component == "template" || component == "all" {
        state.identity.template_version = "earlier-template".into();
    }
    state.identity.starting_binary_revision = "8b7a3c13f".into();
    let mut catalog = fixed_decomposition_catalog("8b7a3c13f").unwrap();
    if component == "catalog" || component == "all" {
        catalog
            .capabilities
            .get_mut("freeze-acceptance")
            .unwrap()
            .timeout_secs = 1500;
        catalog.recompute_digest().unwrap();
    }
    state.identity.catalog_digest = catalog.digest.clone();
    store
        .write_run_json(&run_id, FIXED_DECOMPOSITION_STATE_PATH, &state)
        .unwrap();
    store
        .write_run_json(&run_id, FIXED_CATALOG_PATH, &catalog)
        .unwrap();
    let arguments: serde_json::Value =
        read_json(&store.run_dir(&run_id).join(FIXED_ARGUMENTS_PATH));
    let route: super::super::workflow_provider_route::TrustedProviderRouteSnapshot =
        read_json(&store.run_dir(&run_id).join(FIXED_PROVIDER_ROUTE_PATH));
    let mut metadata: serde_json::Value =
        read_json(&store.run_dir(&run_id).join(FIXED_GENERATED_METADATA_PATH));
    metadata["fixed_identity"] = serde_json::to_value(&state.identity).unwrap();
    metadata["scaffold_hash"] = serde_json::json!(state.identity.script_digest);
    store
        .write_run_json(&run_id, FIXED_GENERATED_METADATA_PATH, &metadata)
        .unwrap();
    let mut run = store.load_state(&run_id).unwrap();
    run.spec.permissions.insert(
        FIXED_LAUNCH_DIGEST_PERMISSION.into(),
        serde_json::json!(
            fixed_launch_digest(
                &state.identity,
                &arguments,
                &catalog,
                &route,
                &serde_json::from_value(metadata["check_environment_policy"].clone()).unwrap(),
            )
            .unwrap()
        ),
    );
    store.save_state(&run).unwrap();
    WorkflowBundle::create_for_run(
        &store,
        &run,
        &source,
        WorkflowBundleOrigin::GeneratedHarness,
    )
    .unwrap();
    for _ in 0..2 {
        let resume = UpgradeBarrier {
            run_id: run_id.clone(),
            store: store.clone(),
            expected: state.identity.clone(),
            builds: AtomicUsize::new(0),
        };
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
            "{component}: {error:#}"
        );
        assert_eq!(resume.builds.load(Ordering::SeqCst), 1);
    }
    let events = std::fs::read_to_string(store.events_path(&run_id)).unwrap();
    let upgrades: Vec<serde_json::Value> = events
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .filter(|e: &serde_json::Value| e["detail"]["event"] == "decomposition_runtime_upgrade")
        .collect();
    assert_eq!(upgrades.len(), 1, "one durable event per transition");
    // Issue 360: the transition's phase seed, derived once and visible.
    let seeded = events
        .lines()
        .filter(|l| l.contains(crate::command::workflow_decompose_seed::SEED_EVENT))
        .count();
    assert_eq!(seeded, 1, "{component}: one seed event per transition");
    let status = crate::command::workflow_decompose_status::render(&store, &run_id)
        .unwrap()
        .unwrap();
    assert!(
        status.contains("phase_seed: transition=0 record=decomposition/phase-seeds/transition-0-"),
        "{status}"
    );
    assert_eq!(
        upgrades[0]["detail"]["old"]["script_digest"],
        state.identity.script_digest
    );
    assert_eq!(
        upgrades[0]["detail"]["new"]["script_digest"],
        workflow_scaffold_hash(FIXED_SCRIPT_SOURCE)
    );
    assert_eq!(
        upgrades[0]["detail"]["old"]["catalog_digest"],
        catalog.digest
    );
    assert_eq!(
        upgrades[0]["detail"]["new"]["catalog_digest"],
        fixed_decomposition_catalog("8b7a3c13f").unwrap().digest
    );
    assert_eq!(
        upgrades[0]["detail"]["new"]["starting_binary_revision"],
        env!("ARCHON_GIT_HASH")
    );
    assert!(upgrades[0]["ts"].as_str().is_some());
    assert_eq!(
        read_json::<FixedDecompositionStateV1>(
            &store.run_dir(&run_id).join(FIXED_DECOMPOSITION_STATE_PATH)
        )
        .identity,
        state.identity
    );
}

#[tokio::test]
async fn upgrade_358_script_resume() {
    resume_upgraded("script").await;
}
#[tokio::test]
async fn upgrade_358_catalog_resume() {
    resume_upgraded("catalog").await;
}
#[tokio::test]
async fn upgrade_358_template_resume() {
    resume_upgraded("template").await;
}
#[tokio::test]
async fn upgrade_358_combined_resume() {
    resume_upgraded("all").await;
}

async fn unreadable_state(field: &str, value: serde_json::Value) {
    let project = fixture_project();
    let (store, run_id, _) =
        super::workflow_decomposition_drift_tests::launch_and_pause(project.path()).await;
    let mut state: serde_json::Value =
        read_json(&store.run_dir(&run_id).join(FIXED_DECOMPOSITION_STATE_PATH));
    state[field] = value;
    store
        .write_run_json(&run_id, FIXED_DECOMPOSITION_STATE_PATH, &state)
        .unwrap();
    let mut run = store.load_state(&run_id).unwrap();
    run.status = RunStatus::Running;
    store.save_state(&run).unwrap();
    let factory = BarrierFactory::resume(
        project
            .path()
            .canonicalize()
            .map(archon_shell::paths::plain)
            .unwrap(),
    );
    let error = resume_fixed_decomposition_with_factory(
        project.path(),
        &run_id,
        true,
        &launch_config(project.path()),
        &empty_env(),
        &factory,
    )
    .await
    .unwrap_err();
    let message = format!("{error:#}");
    assert_eq!(factory.builds.load(Ordering::SeqCst), 0);
    assert!(
        message.contains(field)
            && message.contains("paused")
            && message.contains("restore")
            && message.contains("migrat"),
        "{message}"
    );
    assert_eq!(store.load_state(&run_id).unwrap().status, RunStatus::Paused);
}

#[tokio::test]
async fn upgrade_358_future_schema_pauses() {
    unreadable_state("schema_version", serde_json::json!(99)).await;
}
#[tokio::test]
async fn upgrade_358_malformed_schema_pauses() {
    unreadable_state("schema_version", serde_json::json!("future")).await;
}
#[tokio::test]
async fn upgrade_358_unknown_phase_pauses() {
    unreadable_state("phase", serde_json::json!("unmapped_phase")).await;
}
#[tokio::test]
async fn upgrade_358_missing_identity_field_pauses() {
    unreadable_state("identity", serde_json::json!({})).await;
}

#[test]
fn upgrade_358_exact_launch_shape_maps_new_runtime() {
    let state: FixedDecompositionStateV1 = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/crates/archon-workflow/tests/fixtures/fixed-decomposition-launch-v1.json"
    )))
    .unwrap();
    assert_eq!(state.identity.starting_binary_revision, "8b7a3c13f");
    let mut current = state.identity.clone();
    current.starting_binary_revision = env!("ARCHON_GIT_HASH").into();
    // Simulate deploying a binary with a JS fix as well as this Rust fix.
    current.script_digest =
        workflow_scaffold_hash(&format!("{FIXED_SCRIPT_SOURCE}\n// upgraded harness"));
    let mut catalog =
        fixed_decomposition_catalog(&state.identity.starting_binary_revision).unwrap();
    catalog
        .capabilities
        .get_mut("freeze-acceptance")
        .unwrap()
        .timeout_secs += 600;
    catalog.recompute_digest().unwrap();
    current.catalog_digest = catalog.digest;
    assert_ne!(current.script_digest, state.identity.script_digest);
    assert_ne!(current.catalog_digest, state.identity.catalog_digest);
    archon_workflow::verify_fixed_resume_identity(&state.identity, &current).unwrap();
    for field in [
        "prd_identity",
        "project_root_identity",
        "task_root_identity",
    ] {
        let mut other = current.clone();
        match field {
            "prd_identity" => other.prd_identity = "/other/prd".into(),
            "project_root_identity" => other.project_root_identity = "/other/project".into(),
            "task_root_identity" => other.task_root_identity = "/other/tasks".into(),
            _ => unreachable!(),
        }
        let message = archon_workflow::verify_fixed_resume_identity(&state.identity, &other)
            .unwrap_err()
            .to_string();
        assert!(
            message.contains(field) && message.contains("restore"),
            "{message}"
        );
    }
}

async fn unreadable_result(change: &str) {
    let project = fixture_project();
    let (store, run_id, _) =
        super::workflow_decomposition_drift_tests::launch_and_pause(project.path()).await;
    let v2 = archon_workflow::WorkflowV2ResultStore::new(store.run_dir(&run_id).join("v2"));
    let record = archon_workflow::WorkflowV2CallRecord::new(
        &run_id,
        archon_workflow::WorkflowV2HostCall {
            id: "acceptance-author-1".into(),
            method: archon_workflow::WorkflowV2HostMethod::Agent,
            write_mode: None,
            options: Default::default(),
        },
        1,
        "hash".into(),
        archon_workflow::WorkflowV2Result::accepted("candidate"),
        Vec::new(),
    );
    v2.save_call_record(&record).unwrap();
    let mut value = serde_json::to_value(&record).unwrap();
    let field = match change {
        "schema" => {
            value["schema_version"] = serde_json::json!("future-result-v9");
            "schema_version"
        }
        "status" => {
            value["status"] = serde_json::json!("future_status");
            "status"
        }
        "call" => {
            value["call"]["method"] = serde_json::json!("future_method");
            "call.method"
        }
        _ => unreachable!(),
    };
    std::fs::write(
        v2.result_path(&record.call.id),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();
    let factory = BarrierFactory::resume(
        project
            .path()
            .canonicalize()
            .map(archon_shell::paths::plain)
            .unwrap(),
    );
    let message = format!(
        "{:#}",
        resume_fixed_decomposition_with_factory(
            project.path(),
            &run_id,
            true,
            &launch_config(project.path()),
            &empty_env(),
            &factory
        )
        .await
        .unwrap_err()
    );
    assert!(
        message.contains(field) && message.contains("paused") && message.contains("restore"),
        "{message}"
    );
    assert_eq!(factory.builds.load(Ordering::SeqCst), 0);
    assert_eq!(store.load_state(&run_id).unwrap().status, RunStatus::Paused);
    assert_eq!(
        std::fs::read(v2.result_path(&record.call.id)).unwrap(),
        serde_json::to_vec(&value).unwrap()
    );
}

#[tokio::test]
async fn upgrade_358_unknown_result_schema_pauses() {
    unreadable_result("schema").await;
}
#[tokio::test]
async fn upgrade_358_unknown_result_status_pauses() {
    unreadable_result("status").await;
}
#[tokio::test]
async fn upgrade_358_unknown_result_method_pauses() {
    unreadable_result("call").await;
}

#[tokio::test]
async fn upgrade_358_missing_schema_pauses() {
    unreadable_state("schema_version", serde_json::Value::Null).await;
}

#[tokio::test]
async fn upgrade_358_unknown_frontier_field_pauses() {
    unreadable_state("future_frontier", serde_json::json!({"generation": 2})).await;
}

async fn unreadable_frozen_chain(field: &str, value: serde_json::Value) {
    let project = fixture_project();
    let (store, run_id, _) =
        super::workflow_decomposition_drift_tests::launch_and_pause(project.path()).await;
    let mut args: serde_json::Value = read_json(&store.run_dir(&run_id).join(FIXED_ARGUMENTS_PATH));
    args["frozenChain"][field] = value;
    store
        .write_run_json(&run_id, FIXED_ARGUMENTS_PATH, &args)
        .unwrap();
    let factory = BarrierFactory::resume(
        project
            .path()
            .canonicalize()
            .map(archon_shell::paths::plain)
            .unwrap(),
    );
    let message = format!(
        "{:#}",
        resume_fixed_decomposition_with_factory(
            project.path(),
            &run_id,
            true,
            &launch_config(project.path()),
            &empty_env(),
            &factory
        )
        .await
        .unwrap_err()
    );
    assert!(
        message.contains(&format!("frozenChain.{field}")) && message.contains("restore"),
        "{message}"
    );
    assert_eq!(factory.builds.load(Ordering::SeqCst), 0);
    assert_eq!(store.load_state(&run_id).unwrap().status, RunStatus::Paused);
}
#[tokio::test]
async fn upgrade_358_unmapped_frozen_acceptance_pauses() {
    unreadable_frozen_chain("acceptance", serde_json::json!("true")).await;
}
#[tokio::test]
async fn upgrade_358_unmapped_frozen_subjects_pauses() {
    unreadable_frozen_chain("subjects", serde_json::json!({})).await;
}
#[tokio::test]
async fn upgrade_358_unmapped_frozen_bodies_pauses() {
    unreadable_frozen_chain("bodies", serde_json::json!([99])).await;
}

struct UpgradeBarrier {
    run_id: String,
    store: WorkflowStore,
    expected: archon_workflow::FixedRunIdentityV1,
    builds: AtomicUsize,
}

#[async_trait::async_trait(?Send)]
impl WorkflowLlmClientFactory for UpgradeBarrier {
    async fn build_client(
        &self,
        request: WorkflowLlmClientRequest,
    ) -> archon_workflow::WorkflowResult<Arc<dyn WorkflowLlmClient>> {
        self.builds.fetch_add(1, Ordering::SeqCst);
        assert_eq!(request.session_id, self.run_id);
        assert_eq!(request.origin, "workflow_decompose_v1");
        assert_eq!(self.store.list_runs()?.len(), 1);
        let run = self.store.load_state(&request.session_id)?;
        assert_eq!(run.status, RunStatus::Paused);
        assert!(
            crate::command::workflow_task_root_reclaim::begin_execution(&self.store, &run.id)
                .is_err()
        );
        let state: FixedDecompositionStateV1 = read_json(
            &self
                .store
                .run_dir(&run.id)
                .join(FIXED_DECOMPOSITION_STATE_PATH),
        );
        assert_eq!(
            state.identity, self.expected,
            "provider construction must preserve launch identity"
        );
        WorkflowBundle::verify(&self.store, &run.id)?;
        Err(archon_workflow::WorkflowError::port(
            "barrier observed; stop before execution",
        ))
    }
}

#[path = "workflow_decomposition_upgrade_admission_tests.rs"]
mod admission;
