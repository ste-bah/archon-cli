use super::*;

#[test]
fn guardian_transports_allowlisted_values_without_persisting_them() {
    let status=std::process::Command::new(std::env::current_exe().unwrap()).args([
        "--exact","command::workflow_live::workflow_live_v2::workflow_run_end_observer_tests::native_tests::environment_tests::allowlisted_environment_reaches_guardian_child",
        "--ignored","--nocapture"
    ]).env("FIXTURE_GUARDIAN_TOKEN","guardian-secret-canary-3f1e").status().unwrap();
    assert!(status.success());
}

#[tokio::test]
#[ignore = "private environment child"]
async fn allowlisted_environment_reaches_guardian_child() {
    let mut fixture = frozen_fixture(vec![criterion(
        "AC-X-001",
        command("test -n \"$FIXTURE_GUARDIAN_TOKEN\" && printf '%s' \"$FIXTURE_GUARDIAN_TOKEN\"".into()),
    )]);
    let repo = tempfile::tempdir().unwrap();
    let git = |args: &[&str]| {
        let o = std::process::Command::new("git")
            .arg("-C")
            .arg(repo.path())
            .args(args)
            .output()
            .unwrap();
        assert!(o.status.success());
        String::from_utf8(o.stdout).unwrap().trim().to_string()
    };
    git(&["init", "-q"]);
    git(&["config", "user.email", "fixture@example.invalid"]);
    git(&["config", "user.name", "fixture"]);
    std::fs::write(repo.path().join("input"), "present").unwrap();
    git(&["add", "."]);
    git(&["commit", "-qm", "source"]);
    let scratch = tempfile::tempdir().unwrap();
    fixture.snapshot.native_execution = Some(serde_json::json!({
        "policy":{"repository":repo.path(),"project":fixture.project.path(),"task_root":fixture.task_root,
        "scratch_parent":scratch.path(),"project_inputs":[],"combined":true,"toolchain_path":"/usr/bin:/bin",
        "environment":{},"environment_allowlist":["FIXTURE_GUARDIAN_TOKEN"],"cargo_seed":null,"timeout_secs":10,"output_bytes":2048,"scratch_bytes":16777216},
        "source_commit":git(&["rev-parse","HEAD"])
    }));
    let run = fixture.store.create_run(finalizer_spec()).unwrap();
    persist_native_terminal(&fixture, &run.id);
    let observer = FixedRunEndAcceptanceObserver::new(fixture.store.clone());
    let outcome = observer
        .observe_async(&context(&fixture, &run.id))
        .await
        .unwrap();
    assert_eq!(outcome.operational_deferral_count, 0);
    assert_eq!(outcome.policy_finding_count, 0);
    assert!(!fixture.project.path().join("result").exists());
    let evidence = fixture
        .store
        .run_dir(&run.id)
        .join("observer/native-observation.json");
    let evidence: serde_json::Value =
        serde_json::from_slice(&std::fs::read(evidence).unwrap()).unwrap();
    assert_eq!(evidence["checks"][0]["exit_code"], 0);
    assert_eq!(evidence["teardown_verified"], true);
    let raw = serde_json::to_string(&evidence).unwrap();
    assert!(!raw.contains("guardian-secret-canary-3f1e"));
    assert_eq!(evidence["host_environment"]["FIXTURE_GUARDIAN_TOKEN"], true);
    assert_eq!(evidence["checks"][0]["stdout"], serde_json::json!(b"[REDACTED]".to_vec()));
}
