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

#[tokio::test]
async fn native_nested_verifier_cannot_pass_when_its_floor_is_missing() {
    let check=AcceptanceCheck::Floor {contract:WorkflowV2DeliverableContract {
        kind:"artifact".into(),artifact_path:"missing.json".into(),typed_verifier_command:Some("test -f input".into()),
        ..Default::default()
    }};
    let mut fixture=frozen_fixture(vec![criterion("AC-X-001",check)]);
    let repo=tempfile::tempdir().unwrap();
    let git=|args:&[&str]| {let o=std::process::Command::new("git").arg("-C").arg(repo.path()).args(args).output().unwrap();assert!(o.status.success());String::from_utf8(o.stdout).unwrap().trim().to_string()};
    git(&["init","-q"]);git(&["config","user.email","fixture@example.invalid"]);git(&["config","user.name","fixture"]);
    std::fs::write(repo.path().join("input"),"present").unwrap();git(&["add","."]);git(&["commit","-qm","source"]);
    let scratch=tempfile::tempdir().unwrap();
    fixture.snapshot.native_execution=Some(serde_json::json!({"policy":{"repository":repo.path(),"project":fixture.project.path(),"task_root":fixture.task_root,"scratch_parent":scratch.path(),"project_inputs":[],"combined":true,"toolchain_path":"/usr/bin:/bin","environment":{},"cargo_seed":null,"timeout_secs":10,"output_bytes":2048,"scratch_bytes":16777216},"source_commit":git(&["rev-parse","HEAD"])}));
    let run=fixture.store.create_run(finalizer_spec()).unwrap();
    let outcome=FixedRunEndAcceptanceObserver::new(fixture.store.clone()).observe_async(&context(&fixture,&run.id)).await.unwrap();
    assert!(outcome.policy_finding_count>0,"command passing cannot replace its declarative prerequisites");
}

#[tokio::test]
#[ignore = "internal parent-death subprocess"]
async fn observation_parent_entry() {
    let path=std::env::var("NATIVE_PARENT_REQUEST").unwrap();
    let request:crate::command::acceptance_scratch_guardian::Request=
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    crate::command::acceptance_scratch_guardian::launch(request).await.unwrap();
}

#[test]
fn killed_observer_parent_leaves_no_managed_group_or_worktree() {
    use std::{process::{Command,Stdio},time::{Duration,Instant}};
    let signals=tempfile::tempdir().unwrap();let ready=signals.path().join("ready");
    let cmd=format!("test -f input && echo $$ > '{}' && sleep 60",ready.display());
    let fixture=frozen_fixture(vec![criterion("AC-X-001",command(cmd))]);
    let repo=tempfile::tempdir().unwrap();
    let git=|args:&[&str]| {let o=Command::new("git").arg("-C").arg(repo.path()).args(args).output().unwrap();assert!(o.status.success());String::from_utf8(o.stdout).unwrap().trim().to_string()};
    git(&["init","-q"]);git(&["config","user.email","fixture@example.invalid"]);git(&["config","user.name","fixture"]);
    std::fs::write(repo.path().join("input"),"present").unwrap();git(&["add","."]);git(&["commit","-qm","source"]);
    let scratch=tempfile::tempdir().unwrap();let evidence=signals.path().join("evidence");
    let pin_path=crate::command::workflow_task_set::acceptance_pin_path(fixture.project.path(),&fixture.task_root);
    let request=crate::command::acceptance_scratch_guardian::Request {
        policy:archon_workflow::acceptance_scratch::ScratchPolicy {
            repository:repo.path().into(),project:fixture.project.path().into(),task_root:fixture.task_root.clone(),
            scratch_parent:scratch.path().into(),project_inputs:vec![],combined:true,toolchain_path:"/usr/bin:/bin".into(),
            environment:Default::default(),cargo_seed:None,timeout_secs:60,output_bytes:2048,scratch_bytes:16777216,
        },source_commit:git(&["rev-parse","HEAD"]),expected_pin_digest:content_digest(&std::fs::read(&pin_path).unwrap()),pin_path,evidence:evidence.clone(),
    };
    let request_path=signals.path().join("request.json");std::fs::write(&request_path,serde_json::to_vec(&request).unwrap()).unwrap();
    let mut parent=Command::new(std::env::current_exe().unwrap())
        .args(["--exact","command::workflow_live::workflow_live_v2::workflow_run_end_observer_tests::native_tests::observation_parent_entry","--ignored","--nocapture"])
        .env("NATIVE_PARENT_REQUEST",&request_path).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
    let deadline=Instant::now()+Duration::from_secs(15);
    while !ready.exists() && Instant::now()<deadline {assert!(parent.try_wait().unwrap().is_none(),"parent failed before command readiness");std::thread::sleep(Duration::from_millis(10));}
    if !ready.exists() {let _=parent.kill();panic!("command readiness deadline");}
    let pgid:i32=std::fs::read_to_string(&ready).unwrap().trim().parse().unwrap();
    parent.kill().unwrap();parent.wait().unwrap();
    let record=evidence.join("observation.json");
    while !record.exists() && Instant::now()<deadline {std::thread::sleep(Duration::from_millis(10));}
    assert!(record.exists(),"guardian did not finish after parent SIGKILL");
    let result:archon_workflow::acceptance_scratch::ObservationResult=serde_json::from_slice(&std::fs::read(record).unwrap()).unwrap();
    assert!(result.teardown_verified);assert!(!result.passed());
    assert!(result.checks[0].operational_error.as_ref().unwrap().contains("parent"));
    assert_eq!(unsafe{libc::kill(-pgid,0)},-1,"managed process group survived");
    assert_eq!(std::io::Error::last_os_error().raw_os_error(),Some(libc::ESRCH));
    assert_eq!(std::fs::read_dir(scratch.path()).unwrap().count(),0);
    assert_eq!(git(&["worktree","list","--porcelain"]).matches("worktree ").count(),1);
}

#[tokio::test]
async fn native_policy_on_commandless_contract_keeps_existing_floor_evaluation() {
    let mut fixture=frozen_fixture(vec![criterion("AC-X-001",floor("missing.json"))]);
    fixture.snapshot.native_execution=Some(serde_json::json!({"capture_error":"unused native configuration"}));
    let run=fixture.store.create_run(finalizer_spec()).unwrap();
    let outcome=FixedRunEndAcceptanceObserver::new(fixture.store.clone()).observe_async(&context(&fixture,&run.id)).await.unwrap();
    assert_eq!(outcome.policy_finding_count,1);
    assert_eq!(outcome.evaluated_floor_count,1);
}

#[tokio::test]
async fn native_dispatch_refuses_before_terminal_persistence() {
    let mut fixture=frozen_fixture(vec![criterion("AC-X-001",command("test -f input".into()))]);
    fixture.snapshot.native_execution=Some(serde_json::json!({"capture_error":"must not reach policy parsing"}));
    let run=fixture.store.create_run(finalizer_spec()).unwrap();
    let error=FixedRunEndAcceptanceObserver::new(fixture.store.clone()).observe_async(&context(&fixture,&run.id)).await.unwrap_err();
    assert!(error.to_string().contains("terminal"),"{error}");
}
