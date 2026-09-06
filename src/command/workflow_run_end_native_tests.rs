use super::*;

#[tokio::test]
async fn enabled_native_observer_executes_pinned_command_in_scratch() {
    let mut fixture=frozen_fixture(vec![criterion("AC-X-001",command("test -f input && printf checked > result".into()))]);
    let repo=tempfile::tempdir().unwrap();
    let git=|args:&[&str]| {let o=std::process::Command::new("git").arg("-C").arg(repo.path()).args(args).output().unwrap();assert!(o.status.success());String::from_utf8(o.stdout).unwrap().trim().to_string()};
    git(&["init","-q"]);git(&["config","user.email","fixture@example.invalid"]);git(&["config","user.name","fixture"]);
    std::fs::write(repo.path().join("input"),"present").unwrap();git(&["add","."]);git(&["commit","-qm","source"]);
    let scratch=tempfile::tempdir().unwrap();
    fixture.snapshot.native_execution=Some(serde_json::json!({
        "policy":{"repository":repo.path(),"project":fixture.project.path(),"task_root":fixture.task_root,
        "scratch_parent":scratch.path(),"project_inputs":[],"combined":true,"toolchain_path":"/usr/bin:/bin",
        "environment":{},"cargo_seed":null,"timeout_secs":10,"output_bytes":2048,"scratch_bytes":16777216},
        "source_commit":git(&["rev-parse","HEAD"])
    }));
    let run=fixture.store.create_run(finalizer_spec()).unwrap();
    let observer=FixedRunEndAcceptanceObserver::new(fixture.store.clone());
    let outcome=observer.observe_async(&context(&fixture,&run.id)).await.unwrap();
    assert_eq!(outcome.operational_deferral_count,0);
    assert_eq!(outcome.policy_finding_count,0);
    assert!(!fixture.project.path().join("result").exists());
    let evidence=fixture.store.run_dir(&run.id).join("observer/native-observation.json");
    let evidence:serde_json::Value=serde_json::from_slice(&std::fs::read(evidence).unwrap()).unwrap();
    assert_eq!(evidence["checks"][0]["exit_code"],0);
    assert_eq!(evidence["teardown_verified"],true);
}
