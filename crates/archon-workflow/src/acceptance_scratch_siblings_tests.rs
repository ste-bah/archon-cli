use super::*;

#[path = "acceptance_scratch_siblings_fault_tests.rs"]
mod fault_tests;
#[path = "acceptance_scratch_siblings_internal_tests.rs"]
mod internal_tests;
#[path = "acceptance_scratch_siblings_target_tests.rs"]
mod target_tests;

/// The revisions of a copy in which every named sibling was copied.
fn copied_all(
    copied: WorkflowResult<Copied>,
) -> Vec<crate::acceptance_sibling_paths::SiblingRevision> {
    let copied = copied.unwrap();
    assert!(copied.failures.is_empty(), "{:?}", copied.failures);
    copied.revisions
}

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

#[cfg(unix)]
#[test]
fn scratch_combined_project_materializes_repository_sibling_for_manifest_check() {
    use crate::acceptance_scratch::ScratchPolicy;
    use std::collections::BTreeMap;

    let parent = tempfile::tempdir().unwrap();
    let project = parent.path().join("project");
    let repository = parent.path().join("repo-dir");
    let tasks = project.join("tasks");
    let scratch = parent.path().join("scratch");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(&tasks).unwrap();
    std::fs::create_dir_all(project.join("data")).unwrap();
    std::fs::write(project.join("data/input.txt"), "fixture\n").unwrap();
    std::fs::create_dir_all(repository.join("crates/demo/src")).unwrap();
    std::fs::write(
        repository.join("Cargo.toml"),
        "[workspace]\nmembers = [\"crates/demo\"]\nresolver = \"2\"\n",
    )
    .unwrap();
    std::fs::write(
        repository.join("crates/demo/Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(repository.join("crates/demo/src/main.rs"), "fn main() {}\n").unwrap();
    run_git(&repository, &["init", "-q"]);
    run_git(
        &repository,
        &["config", "user.email", "test@example.invalid"],
    );
    run_git(&repository, &["config", "user.name", "test"]);
    run_git(&repository, &["add", "."]);
    run_git(&repository, &["commit", "-qm", "tree"]);
    let commit = git_head(&repository).unwrap();

    let check = "cargo test --manifest-path ../repo-dir/Cargo.toml --no-run".to_string();
    let baseline_project = scratch.join("pre-implementation/project");
    std::fs::create_dir_all(&baseline_project).unwrap();
    let before = std::process::Command::new("cargo")
        .args([
            "metadata",
            "--manifest-path",
            "../repo-dir/Cargo.toml",
            "--no-deps",
        ])
        .current_dir(&baseline_project)
        .output()
        .unwrap();
    let before_error = String::from_utf8_lossy(&before.stderr);
    println!(
        "PRE-MATERIALIZATION cargo evidence: {}",
        before_error.trim()
    );
    assert!(!before.status.success());
    assert!(before_error.contains("manifest path `../repo-dir/Cargo.toml` does not exist"));

    let policy = ScratchPolicy {
        repository: repository.clone(),
        project: project.clone(),
        task_root: tasks,
        scratch_parent: scratch,
        project_inputs: vec![PathBuf::from("data")],
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
    let roots = super::ScratchRoots::prepare_inner_with_checks(
        &policy,
        &commit,
        &[(check, crate::task_set_contract::TrustedCwd::ProjectRoot)],
    )
    .unwrap();
    let manifest = roots.project().join("../repo-dir/Cargo.toml");
    assert!(
        manifest.is_file(),
        "scratch manifest missing: {}",
        manifest.display()
    );
    let after = std::process::Command::new("cargo")
        .args([
            "metadata",
            "--manifest-path",
            "../repo-dir/Cargo.toml",
            "--no-deps",
        ])
        .current_dir(roots.project())
        .output()
        .unwrap();
    assert!(
        after.status.success(),
        "scratch cargo metadata failed: {}",
        String::from_utf8_lossy(&after.stderr)
    );
    assert!(roots.project().join("data/input.txt").is_file());
    drop(roots);
}

#[test]
fn refuses_project_root_and_ancestor_sources() {
    let project = Path::new("/work/project");
    for source in ["/work/project", "/work", "/"] {
        assert!(
            is_project_or_ancestor(Path::new(source), project),
            "{source}"
        );
    }
    assert!(!is_project_or_ancestor(Path::new("/work/sibling"), project));
}

#[test]
fn configured_copy_keeps_nested_tracked_sibling_and_omits_build_output() {
    // EXPECTED: `../a/b` runs tracked status 23 from the configured copy;
    // its untracked target tree is absent.
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    let copied = root.path().join("scratch/project");
    let sibling = root.path().join("a/b");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(&sibling).unwrap();
    std::fs::create_dir_all(&copied).unwrap();
    std::fs::write(
        sibling.join("check.sh"),
        "#!/bin/sh\n# current committed revision\nexit 23\n",
    )
    .unwrap();
    run_git(&sibling, &["init", "-q"]);
    run_git(&sibling, &["config", "user.email", "test@example.invalid"]);
    run_git(&sibling, &["config", "user.name", "test"]);
    run_git(&sibling, &["add", "check.sh"]);
    run_git(&sibling, &["commit", "-q", "-m", "tracked"]);
    std::fs::write(sibling.join("check.sh"), "#!/bin/sh\nexit 23\n").unwrap();
    run_git(&sibling, &["add", "check.sh"]);
    run_git(&sibling, &["commit", "-q", "-m", "current head"]);
    let expected_head = git_head(&sibling).unwrap();
    std::fs::create_dir_all(sibling.join("target/debug")).unwrap();
    std::fs::write(sibling.join("target/debug/output"), b"untracked").unwrap();
    let mut remaining = u64::MAX;
    let evidence = copied_all(copy_external(
        &project,
        &project,
        &copied,
        &copied,
        &[(
            "cd ../a/b && sh check.sh".into(),
            crate::task_set_contract::TrustedCwd::ProjectRoot,
        )],
        &mut remaining,
        false,
    ));
    #[cfg(unix)]
    let command = std::process::Command::new("/bin/sh")
        .args(["-c", "cd ../a/b && sh check.sh"])
        .current_dir(&copied)
        .output()
        .unwrap();
    #[cfg(unix)]
    {
        println!(
            "EVIDENCE #420 scratch-sibling BEFORE: sibling absent; AFTER: tracked sibling status={:?}; target copied={}",
            command.status.code(),
            copied.parent().unwrap().join("a/b/target").exists()
        );
        assert_eq!(command.status.code(), Some(23));
    }
    assert!(!copied.parent().unwrap().join("a/b/target").exists());
    let copied_head = git_head(&copied.parent().unwrap().join("a/b")).unwrap();
    println!(
        "EVIDENCE #420 scratch-sibling-revision BEFORE: old revision could be copied; AFTER: path=../a/b, source={expected_head}, copied={copied_head}"
    );
    assert_eq!(copied_head, expected_head);
    assert_eq!(evidence[0].path, PathBuf::from("../a/b"));
    assert_eq!(evidence[0].commit.as_deref(), Some(expected_head.as_str()));
}

#[cfg(unix)]
#[test]
fn repository_sibling_alias_uses_the_scratch_tree_commit() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    let repository = root.path().join("repo-dir");
    let copied_project = root.path().join("scratch/project");
    let copied_repository = root.path().join("scratch/repo");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(&repository).unwrap();
    std::fs::create_dir_all(&copied_project).unwrap();
    run_git(&repository, &["init", "-q"]);
    run_git(
        &repository,
        &["config", "user.email", "test@example.invalid"],
    );
    run_git(&repository, &["config", "user.name", "test"]);
    std::fs::write(
        repository.join("Cargo.toml"),
        "[package]\nname=\"tree\"\nversion=\"0.1.0\"\nedition=\"2021\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(repository.join("src")).unwrap();
    std::fs::write(repository.join("src/lib.rs"), "pub fn tree() {}\n").unwrap();
    run_git(&repository, &["add", "."]);
    run_git(&repository, &["commit", "-qm", "manifest at tree commit"]);
    let tree_commit = git_head(&repository).unwrap();
    std::fs::write(
        repository.join("Cargo.toml"),
        "[package]\nname=\"live_manifest\"\nversion=\"0.1.0\"\nedition=\"2021\"\n",
    )
    .unwrap();
    run_git(&repository, &["add", "-A"]);
    run_git(
        &repository,
        &["commit", "-qm", "manifest changed at live head"],
    );

    let cloned = std::process::Command::new("git")
        .args(["clone", "--quiet", "--shared", "--no-checkout", "--"])
        .arg(&repository)
        .arg(&copied_repository)
        .output()
        .unwrap();
    assert!(
        cloned.status.success(),
        "{}",
        String::from_utf8_lossy(&cloned.stderr)
    );
    run_git(
        &copied_repository,
        &["checkout", "--quiet", "--detach", &tree_commit],
    );

    let live_check = std::process::Command::new("/bin/sh")
        .args([
            "-c",
            "test -f ../repo-dir/Cargo.toml && grep -q tree ../repo-dir/Cargo.toml",
        ])
        .current_dir(&project)
        .output()
        .unwrap();
    assert!(!live_check.status.success());

    let mut remaining = u64::MAX;
    let evidence = copied_all(copy_external(
        &project,
        &repository,
        &copied_project,
        &copied_repository,
        &[(
            "cargo metadata --manifest-path ../repo-dir/Cargo.toml".into(),
            crate::task_set_contract::TrustedCwd::ProjectRoot,
        )],
        &mut remaining,
        false,
    ));
    let alias = copied_project.join("../repo-dir");
    assert!(alias.join("Cargo.toml").is_file());
    assert_eq!(
        std::fs::canonicalize(alias.join("Cargo.toml")).unwrap(),
        std::fs::canonicalize(copied_repository.join("Cargo.toml")).unwrap()
    );
    let check = std::process::Command::new("/bin/sh")
        .args([
            "-c",
            "test -f ../repo-dir/Cargo.toml && grep -q tree ../repo-dir/Cargo.toml",
        ])
        .current_dir(&copied_project)
        .output()
        .unwrap();
    assert!(check.status.success());
    assert_eq!(git_head(&alias).unwrap(), tree_commit);
    assert_eq!(evidence[0].commit.as_deref(), Some(tree_commit.as_str()));
}

#[test]
fn a_check_without_an_outside_path_keeps_the_scratch_parent_unchanged() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    let copied = root.path().join("scratch/project");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(&copied).unwrap();
    let before = std::fs::read_dir(copied.parent().unwrap()).unwrap().count();
    let mut remaining = u64::MAX;
    copied_all(copy_external(
        &project,
        &project,
        &copied,
        &copied,
        &[(
            "cargo test".into(),
            crate::task_set_contract::TrustedCwd::ProjectRoot,
        )],
        &mut remaining,
        false,
    ));
    let after = std::fs::read_dir(copied.parent().unwrap()).unwrap().count();
    assert_eq!(before, after);
}

#[test]
fn sibling_discovery_parses_quotes_and_ignores_prose() {
    // EXPECTED before running: the scanner returns the quoted manifest
    // sibling and does not treat an echo argument as a required path.
    assert_eq!(
        crate::acceptance_sibling_paths::scan("cargo test --manifest-path=\"../repo/Cargo.toml\"")
            .into_iter()
            .map(|s| s.path)
            .collect::<Vec<_>>(),
        vec![PathBuf::from("../repo/Cargo.toml")]
    );
    assert!(!crate::acceptance_sibling_paths::scan("echo \"../missing\"")[0].required);
}

#[test]
fn copies_a_standalone_sibling_file_and_omits_nested_build_output() {
    // EXPECTED before implementation: the file is copied in-place while
    // nested target and node_modules trees are omitted.
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    let copied = root.path().join("scratch/project");
    let sibling = root.path().join("sibling");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(&copied).unwrap();
    std::fs::write(root.path().join("note.txt"), "standalone file").unwrap();
    std::fs::create_dir_all(sibling.join("target/debug")).unwrap();
    std::fs::create_dir_all(sibling.join("node_modules/pkg")).unwrap();
    std::fs::write(sibling.join("note.txt"), "real file").unwrap();
    std::fs::write(sibling.join("target/debug/output"), "build").unwrap();
    std::fs::write(sibling.join("node_modules/pkg/output"), "dependency").unwrap();
    let mut remaining = u64::MAX;
    let evidence = copied_all(copy_external(
        &project,
        &project,
        &copied,
        &copied,
        &[(
            "echo ../note.txt ../sibling".into(),
            crate::task_set_contract::TrustedCwd::ProjectRoot,
        )],
        &mut remaining,
        false,
    ));
    let copied_sibling = copied.parent().unwrap().join("sibling");
    assert_eq!(
        std::fs::read_to_string(copied.parent().unwrap().join("note.txt")).unwrap(),
        "standalone file"
    );
    assert_eq!(
        std::fs::read_to_string(copied_sibling.join("note.txt")).unwrap(),
        "real file"
    );
    assert!(!copied_sibling.join("target").exists());
    assert!(!copied_sibling.join("node_modules").exists());
    assert!(
        evidence
            .iter()
            .any(|revision| revision.path.as_path() == std::path::Path::new("../note.txt"))
    );
    assert!(
        evidence
            .iter()
            .any(|revision| revision.path.as_path() == std::path::Path::new("../sibling"))
    );
    assert_eq!(
        evidence
            .iter()
            .find(|revision| revision.path.as_path() == std::path::Path::new("../sibling"))
            .unwrap()
            .commit,
        None
    );
}
