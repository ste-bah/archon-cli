use super::*;

fn run_git(path: &Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

struct TargetFixture {
    _temp: tempfile::TempDir,
    policy: crate::acceptance_scratch::ScratchPolicy,
    commit: String,
    repository: PathBuf,
}

fn target_fixture(tracked_target: bool) -> TargetFixture {
    use crate::acceptance_scratch::ScratchPolicy;
    use std::collections::BTreeMap;

    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project-1");
    let repository = temp.path().join("archon-cli");
    let tasks = project.join("tasks");
    std::fs::create_dir_all(project.join(".archon/trading-lab")).unwrap();
    std::fs::create_dir_all(&tasks).unwrap();
    std::fs::create_dir_all(&repository).unwrap();
    std::fs::write(
        repository.join("Cargo.toml"),
        "[package]\nname=\"fixture\"\nversion=\"0.1.0\"\nedition=\"2021\"\n",
    )
    .unwrap();
    if tracked_target {
        std::fs::create_dir_all(repository.join("target")).unwrap();
        std::fs::write(repository.join("target/committed.txt"), "existing output").unwrap();
    }
    run_git(&repository, &["init", "-q"]);
    run_git(
        &repository,
        &["config", "user.email", "test@example.invalid"],
    );
    run_git(&repository, &["config", "user.name", "test"]);
    run_git(&repository, &["add", "."]);
    run_git(&repository, &["commit", "-qm", "fixture"]);
    let commit = git_head(&repository).unwrap();
    if !tracked_target {
        // An ignored/untracked executable lets sibling discovery see the
        // nested alias while the scratch worktree itself omits build output.
        std::fs::create_dir_all(repository.join("target/release")).unwrap();
        std::fs::write(repository.join("target/release/archon"), b"fixture binary").unwrap();
    }
    let policy = ScratchPolicy {
        repository: repository.clone(),
        project,
        task_root: tasks,
        scratch_parent: temp.path().join("observations"),
        project_inputs: vec![PathBuf::from(".archon/trading-lab")],
        project_input_excludes: vec![],
        combined: true,
        toolchain_path: std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".into()),
        environment: BTreeMap::new(),
        environment_allowlist: vec![],
        cargo_seed: None,
        timeout_secs: 30,
        output_bytes: 4096,
        scratch_bytes: 16 * 1024 * 1024,
        build_cache: None,
    };
    TargetFixture {
        _temp: temp,
        policy,
        commit,
        repository,
    }
}

#[cfg(unix)]
#[test]
fn combined_project_repository_sibling_nested_target_alias_does_not_create_conflict() {
    use crate::acceptance_scratch::ScratchRoots;
    use crate::task_set_contract::TrustedCwd;

    let fixture = target_fixture(false);
    let checks = [
        (
            "cargo metadata --manifest-path ../archon-cli/Cargo.toml --no-deps".into(),
            TrustedCwd::ProjectRoot,
        ),
        (
            "../archon-cli/target/release/archon --version".into(),
            TrustedCwd::ProjectRoot,
        ),
    ];
    let roots = ScratchRoots::prepare_inner_with_checks(&fixture.policy, &fixture.commit, &checks)
        .expect("nested repository alias must reuse the repository copy");
    let alias = roots.project().join("../archon-cli");
    assert_eq!(
        alias.canonicalize().unwrap(),
        roots.repository().canonicalize().unwrap()
    );
    let target = roots.repository().join("target");
    assert!(target.symlink_metadata().unwrap().file_type().is_symlink());
    assert_eq!(std::fs::read_link(target).unwrap(), roots.target());
    assert!(fixture.repository.join("target/release/archon").is_file());
    drop(roots);
}

#[test]
fn combined_view_still_refuses_a_real_preexisting_target() {
    use crate::acceptance_scratch::ScratchRoots;

    let fixture = target_fixture(true);
    let error = match ScratchRoots::prepare_inner_with_checks(&fixture.policy, &fixture.commit, &[])
    {
        Ok(_) => panic!("committed target content must remain a conflict"),
        Err(error) => error,
    };
    assert!(
        error
            .to_string()
            .contains("source/project target path conflicts with scratch build target")
    );
}
