use archon_workflow::acceptance_scratch::ScratchPolicy;
use archon_workflow::acceptance_world::{AcceptanceCommandKind, FrozenCommandRef};
use archon_workflow::task_set_contract::{AcceptanceContract, content_digest};
use std::{collections::BTreeMap, path::Path, process::Command};
fn git(root: &Path, args: &[&str]) -> String {
    let o = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8(o.stdout).unwrap().trim().into()
}
pub fn fixture(
    command: &str,
) -> (
    tempfile::TempDir,
    ScratchPolicy,
    String,
    AcceptanceContract,
    Vec<FrozenCommandRef>,
) {
    let t = tempfile::tempdir().unwrap();
    let repo = t.path().join("repo");
    let project = t.path().join("project");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::create_dir_all(project.join("data")).unwrap();
    std::fs::create_dir_all(project.join("tasks")).unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["config", "user.email", "test@example.invalid"]);
    git(&repo, &["config", "user.name", "test"]);
    std::fs::write(repo.join("input"), "source").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "fixture"]);
    let commit = git(&repo, &["rev-parse", "HEAD"]);
    std::fs::write(project.join("data/value"), "before").unwrap();
    let p = ScratchPolicy {
        repository: repo,
        project: project.clone(),
        task_root: project.join("tasks"),
        scratch_parent: t.path().join("scratch"),
        project_inputs: vec!["data".into()],
        combined: true,
        toolchain_path: "/usr/bin:/bin:/usr/sbin:/sbin".into(),
        environment: BTreeMap::new(),
        cargo_seed: None,
        timeout_secs: 3,
        output_bytes: 2048,
        scratch_bytes: 16 * 1024 * 1024,
    };
    let c:AcceptanceContract=serde_json::from_value(serde_json::json!({"schema_version":1,"prd":{"path":"p","digest":"d"},"gap_policy":{"permitted_acceptance_ids":[],"forbidden_phrases":[],"required_fields":[]},"acceptance":[{"id":"AC-X-001","criterion":"output correct","check":{"kind":"command","command":command,"cwd":"project_root"},"judgment":{"verdict":"accepted","counterexample":"incorrect output","reason":"checks output","host_call_id":"j"}}]})).unwrap();
    let refs = vec![FrozenCommandRef {
        acceptance_id: "AC-X-001".into(),
        kind: AcceptanceCommandKind::Command,
        chain_digest: "chain".into(),
        command_digest: content_digest(command.as_bytes()),
    }];
    (t, p, commit, c, refs)
}
