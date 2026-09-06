use archon_workflow::acceptance_scratch::{ScratchPolicy, observe_commands_cancellable};
use archon_workflow::acceptance_world::{AcceptanceCommandKind, FrozenCommandRef};
use archon_workflow::task_set_contract::{AcceptanceContract, content_digest};
use std::{
    collections::BTreeMap,
    process::Command,
    sync::{Arc, atomic::AtomicBool},
};
#[tokio::test]
async fn supervisor_cancellation_reaps_work_before_root_cleanup() {
    let t = tempfile::tempdir().unwrap();
    let repo = t.path().join("repo");
    let project = t.path().join("project");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::create_dir_all(project.join("tasks")).unwrap();
    let git = |args: &[&str]| {
        let o = Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(args)
            .output()
            .unwrap();
        assert!(o.status.success());
        String::from_utf8(o.stdout).unwrap().trim().to_string()
    };
    git(&["init", "-q"]);
    git(&["config", "user.email", "fixture@example.invalid"]);
    git(&["config", "user.name", "fixture"]);
    std::fs::write(repo.join("input"), "x").unwrap();
    git(&["add", "."]);
    git(&["commit", "-qm", "fixture"]);
    let commit = git(&["rev-parse", "HEAD"]);
    let p = ScratchPolicy {
        repository: repo,
        project: project.clone(),
        task_root: project.join("tasks"),
        scratch_parent: t.path().join("scratch"),
        project_inputs: vec![],
        combined: true,
        toolchain_path: "/usr/bin:/bin".into(),
        environment: BTreeMap::new(),
        cargo_seed: None,
        timeout_secs: 30,
        output_bytes: 1024,
        scratch_bytes: 1024 * 1024,
    };
    let command = "sleep 30; test -f input";
    let c:AcceptanceContract=serde_json::from_value(serde_json::json!({"schema_version":1,"prd":{"path":"p","digest":"d"},"gap_policy":{"permitted_acceptance_ids":[],"forbidden_phrases":[],"required_fields":[]},"acceptance":[{"id":"AC-X-001","criterion":"valid","check":{"kind":"command","command":command,"cwd":"project_root"},"judgment":{"verdict":"accepted","counterexample":"missing","reason":"fails missing","host_call_id":"j"}}]})).unwrap();
    let refs = [FrozenCommandRef {
        acceptance_id: "AC-X-001".into(),
        kind: AcceptanceCommandKind::Command,
        chain_digest: "chain".into(),
        command_digest: content_digest(command.as_bytes()),
    }];
    let cancel = Arc::new(AtomicBool::new(true));
    let result = observe_commands_cancellable(
        &p,
        &commit,
        &c,
        "chain",
        &refs,
        &t.path().join("evidence"),
        cancel,
    )
    .await
    .unwrap();
    assert!(!result.passed());
    assert!(result.teardown_verified);
    assert!(
        result.checks[0]
            .operational_error
            .as_ref()
            .unwrap()
            .contains("parent")
    );
    assert_eq!(std::fs::read_dir(&p.scratch_parent).unwrap().count(), 0);
}
