use super::*;

async fn seeded_cancelled_run() -> (
    tempfile::TempDir,
    WorkflowStore,
    archon_workflow::WorkflowRun,
) {
    let project = fixture_project();
    let factory = BarrierFactory::launch(project.path().canonicalize().unwrap());
    run_fixed_decomposition_with_factory(
        project.path(),
        Path::new("prds/PRD-X.md"),
        Path::new("tasks/PRD-X"),
        None,
        true,
        &launch_config(project.path()),
        &empty_env(),
        &factory,
    )
    .await
    .unwrap_err();
    let store = WorkflowStore::project(project.path().canonicalize().unwrap());
    let run = store.list_runs().unwrap().pop().unwrap();
    assert_eq!(run.status, RunStatus::Cancelled);
    (project, store, run)
}

async fn resume_must_refuse_before_provider(
    project: &Path,
    run_id: &str,
    factory: &PanicFactory,
) -> String {
    resume_fixed_decomposition_with_factory(
        project,
        run_id,
        true,
        &launch_config(project),
        &empty_env(),
        factory,
    )
    .await
    .unwrap_err()
    .to_string()
}

#[tokio::test]
async fn fixed_launch_phase_zero_refuses_zero_malformed_and_duplicate_obligations() {
    for (prd, needle) in [
        ("# Empty PRD\n", "zero obligations"),
        (
            "# Bad PRD\n\n## Requirements\n- REQ-bad-1 invalid\n\n## Acceptance\n| ID | Criterion |\n|---|---|\n| AC-X-001 | valid |\n",
            "malformed",
        ),
        (
            "# Duplicate PRD\n\n## Requirements\n| ID | Requirement |\n|---|---|\n| REQ-X-001 | one |\n| REQ-X-001 | two |\n\n## Acceptance\n| ID | Criterion |\n|---|---|\n| AC-X-001 | valid |\n",
            "more than one",
        ),
    ] {
        let project = fixture_project();
        std::fs::write(project.path().join("prds/PRD-X.md"), prd).unwrap();
        let factory = PanicFactory {
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
            &factory,
        )
        .await
        .unwrap_err();

        assert!(error.to_string().contains(needle), "{error:#}");
        assert_eq!(factory.builds.load(Ordering::SeqCst), 0);
        assert!(!project.path().join(".archon/workflows").exists());
    }
}

#[tokio::test]
async fn fixed_resume_refuses_same_path_prd_content_mutation() {
    let (project, store, run) = seeded_cancelled_run().await;
    std::fs::write(
        project.path().join("prds/PRD-X.md"),
        "# PRD X changed\n\n## Requirements\n\nREQ-X-001: changed.\n\n## Acceptance\n| ID | Criterion |\n|---|---|\n| AC-X-001 | changed |\n",
    )
    .unwrap();
    let factory = PanicFactory {
        builds: AtomicUsize::new(0),
    };

    let error = resume_must_refuse_before_provider(project.path(), &run.id, &factory).await;

    assert!(
        error.contains("arguments") || error.contains("launch snapshot"),
        "{error}"
    );
    assert_eq!(factory.builds.load(Ordering::SeqCst), 0);
    assert_eq!(
        store.load_state(&run.id).unwrap().status,
        RunStatus::Cancelled
    );
}

#[tokio::test]
async fn fixed_resume_refuses_mutated_arguments_catalog_route_and_metadata() {
    for (relative, mutate, needle) in [
        (
            FIXED_ARGUMENTS_PATH,
            serde_json::json!({"projectRoot":"/other","prdPath":"/other/prd","taskRoot":"/other/tasks","gateMode":"observe"}),
            "arguments",
        ),
        (
            FIXED_CATALOG_PATH,
            serde_json::json!({"schema_version":1,"starting_binary_revision":"changed","digest":"changed","capabilities":{}}),
            "catalog",
        ),
        (
            crate::command::workflow_decompose::FIXED_PROVIDER_ROUTE_PATH,
            serde_json::json!({"origin":"trusted_config","endpoint":"https://changed.invalid","endpoint_digest":"changed"}),
            "provider route",
        ),
        (
            crate::command::workflow_decompose::FIXED_GENERATED_METADATA_PATH,
            serde_json::json!({"schema_version":"workflow-generated-v2-metadata-v1","run_kind":"fixed_decomposition_v1","script_lifecycle":false}),
            "generated metadata",
        ),
    ] {
        let (project, store, run) = seeded_cancelled_run().await;
        store.write_run_json(&run.id, relative, &mutate).unwrap();
        let factory = PanicFactory {
            builds: AtomicUsize::new(0),
        };

        let error = resume_must_refuse_before_provider(project.path(), &run.id, &factory).await;

        assert!(error.contains(needle), "{relative}: {error}");
        assert_eq!(factory.builds.load(Ordering::SeqCst), 0);
        assert_eq!(
            store.load_state(&run.id).unwrap().status,
            RunStatus::Cancelled
        );
    }
}

#[tokio::test]
async fn fixed_resume_requires_bundle_anchored_launch_digest() {
    let (project, store, run) = seeded_cancelled_run().await;
    let compiled = store
        .run_dir(&run.id)
        .join(archon_workflow::bundle::COMPILED_SPEC_FILE);
    let mut spec: archon_workflow::WorkflowSpec =
        serde_yaml_ng::from_str(&std::fs::read_to_string(&compiled).unwrap()).unwrap();
    spec.permissions.insert(
        crate::command::workflow_decompose::FIXED_LAUNCH_DIGEST_PERMISSION.into(),
        serde_json::Value::String("0".repeat(64)),
    );
    std::fs::write(&compiled, spec.to_yaml().unwrap()).unwrap();
    let factory = PanicFactory {
        builds: AtomicUsize::new(0),
    };

    let error = resume_must_refuse_before_provider(project.path(), &run.id, &factory).await;

    assert!(
        error.contains("compiled workflow") || error.contains("bundle"),
        "{error}"
    );
    assert_eq!(factory.builds.load(Ordering::SeqCst), 0);
}
