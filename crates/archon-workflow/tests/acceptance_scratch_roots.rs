use archon_workflow::acceptance_scratch::{ScratchPolicy, ScratchRoots};
use std::{collections::BTreeMap, path::Path, process::Command};
fn git(root: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}
#[test]
fn combined_scratch_uses_recorded_commit_private_data_and_relative_target() {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("repo");
    let project = temp.path().join("project");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::create_dir_all(project.join("data")).unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["config", "user.email", "fixture@example.invalid"]);
    git(&repo, &["config", "user.name", "fixture"]);
    std::fs::write(repo.join("source.txt"), "committed").unwrap();
    git(&repo, &["add", "source.txt"]);
    git(&repo, &["commit", "-qm", "fixture"]);
    let commit = git(&repo, &["rev-parse", "HEAD"]);
    std::fs::write(repo.join("source.txt"), "dirty-live").unwrap();
    std::fs::write(project.join("data/input.txt"), "live-input").unwrap();
    let policy = ScratchPolicy {
        repository: repo.clone(),
        project: project.clone(),
        task_root: project.join("tasks"),
        scratch_parent: temp.path().join("observations"),
        project_inputs: vec!["data".into()],
        combined: true,
        toolchain_path: "/usr/bin:/bin".into(),
        environment: BTreeMap::new(),
        cargo_seed: None,
        timeout_secs: 30,
        output_bytes: 4096,
        scratch_bytes: 16 * 1024 * 1024,
    };
    std::fs::create_dir_all(&policy.task_root).unwrap();
    let mut roots = ScratchRoots::prepare(&policy, &commit).unwrap();
    assert_eq!(
        std::fs::read_to_string(roots.project().join("source.txt")).unwrap(),
        "committed"
    );
    assert_eq!(
        roots.project().join("target").canonicalize().unwrap(),
        roots.target()
    );
    std::fs::write(roots.project().join("data/input.txt"), "scratch-change").unwrap();
    assert_eq!(
        std::fs::read_to_string(project.join("data/input.txt")).unwrap(),
        "live-input"
    );
    let path = roots.root().to_path_buf();
    roots.cleanup().unwrap();
    assert!(!path.exists());
    assert_eq!(
        std::fs::read_to_string(repo.join("source.txt")).unwrap(),
        "dirty-live"
    );
    assert_eq!(
        git(&repo, &["worktree", "list", "--porcelain"])
            .matches("worktree ")
            .count(),
        1
    );
}
#[test]
fn unsafe_input_and_credential_environment_are_rejected_before_setup() {
    let mut p = ScratchPolicy {
        repository: "/repo".into(),
        project: "/project".into(),
        task_root: "/project/tasks".into(),
        scratch_parent: "/scratch".into(),
        project_inputs: vec!["../escape".into()],
        combined: false,
        toolchain_path: "/usr/bin:/bin".into(),
        environment: BTreeMap::new(),
        cargo_seed: None,
        timeout_secs: 1,
        output_bytes: 1024,
        scratch_bytes: 1024,
    };
    assert!(p.validate().is_err());
    p.project_inputs = vec!["data".into()];
    p.environment
        .insert("ANTHROPIC_API_KEY".into(), "canary".into());
    assert!(p.validate().is_err());
    p.environment.clear();
    p.environment
        .insert("BASH_ENV".into(), "/tmp/inject".into());
    assert!(p.validate().is_err());
}

#[test]
fn combined_view_preserves_committed_cargo_configuration() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    let project = tmp.path().join("project");
    std::fs::create_dir_all(repo.join(".cargo")).unwrap();
    std::fs::create_dir_all(project.join("tasks")).unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["config", "user.email", "fixture@example.invalid"]);
    git(&repo, &["config", "user.name", "fixture"]);
    std::fs::write(repo.join(".cargo/config.toml"), "[build]\njobs=1\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "fixture"]);
    let p = ScratchPolicy {
        repository: repo.clone(),
        project: project.clone(),
        task_root: project.join("tasks"),
        scratch_parent: tmp.path().join("scratch"),
        project_inputs: vec![],
        combined: true,
        toolchain_path: "/usr/bin:/bin".into(),
        environment: BTreeMap::new(),
        cargo_seed: None,
        timeout_secs: 30,
        output_bytes: 4096,
        scratch_bytes: 1024 * 1024,
    };
    let mut roots = ScratchRoots::prepare(&p, &git(&repo, &["rev-parse", "HEAD"])).unwrap();
    assert_eq!(
        std::fs::read_to_string(roots.project().join(".cargo/config.toml")).unwrap(),
        "[build]\njobs=1\n"
    );
    roots.cleanup().unwrap();
}

#[test]
fn directly_selected_credential_file_is_not_exported() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    let project = tmp.path().join("project");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::create_dir_all(project.join("tasks")).unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["config", "user.email", "fixture@example.invalid"]);
    git(&repo, &["config", "user.name", "fixture"]);
    std::fs::write(repo.join("source"), "x").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "fixture"]);
    std::fs::write(project.join("credentials.toml"), "secret").unwrap();
    let p = ScratchPolicy {
        repository: repo.clone(),
        project: project.clone(),
        task_root: project.join("tasks"),
        scratch_parent: tmp.path().join("scratch"),
        project_inputs: vec!["credentials.toml".into()],
        combined: false,
        toolchain_path: "/usr/bin:/bin".into(),
        environment: BTreeMap::new(),
        cargo_seed: None,
        timeout_secs: 1,
        output_bytes: 1024,
        scratch_bytes: 1024 * 1024,
    };
    assert!(ScratchRoots::prepare(&p, &git(&repo, &["rev-parse", "HEAD"])).is_err());
}
