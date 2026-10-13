//! Issue 440: a repository nested inside a project input is skipped by one
//! rule in the scratch copy, the hermetic copy and the probe digest, so a
//! check never reads a tree the probe key does not bind.
use std::collections::{BTreeMap, BTreeSet};

use archon_workflow::acceptance_scratch::{ScratchPolicy, ScratchRoots};

use super::*;

fn git(path: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {}: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn write(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

/// A repository at `path` with `tracked.txt` committed; returns its HEAD.
fn commit_repository(path: &Path) -> String {
    std::fs::create_dir_all(path).unwrap();
    git(path, &["init", "-q"]);
    git(path, &["config", "user.email", "test@example.invalid"]);
    git(path, &["config", "user.name", "test"]);
    std::fs::write(path.join("tracked.txt"), b"committed").unwrap();
    git(path, &["add", "tracked.txt"]);
    git(path, &["commit", "-q", "-m", "fixture"]);
    git(path, &["rev-parse", "HEAD"])
}

/// Every regular file under `root`, project-relative, '/'-separated.
fn files(root: &Path) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let mut stack = vec![PathBuf::new()];
    while let Some(relative) = stack.pop() {
        for entry in std::fs::read_dir(root.join(&relative)).unwrap() {
            let entry = entry.unwrap();
            let child = relative.join(entry.file_name());
            let kind = entry.file_type().unwrap();
            if kind.is_dir() {
                stack.push(child);
            } else if kind.is_file() {
                found.insert(relative_key(&child));
            }
        }
    }
    found
}

struct Fixture {
    _temp: tempfile::TempDir,
    project: PathBuf,
    repository: PathBuf,
    commit: String,
    policy: ScratchPolicy,
}

/// Project `project` with input `work`; the run's repository is
/// `work/repo` (inside the input), plus a vendored repository
/// `work/vendor` that is not the run's.
fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let base = archon_shell::paths::canonicalize(temp.path()).unwrap();
    let project = base.join("project");
    let repository = project.join("work/repo");
    let commit = commit_repository(&repository);
    write(&repository.join("live-only.txt"), b"working tree only");
    write(&project.join("work/data.txt"), b"project data");
    write(&project.join("work/vendor/lib.txt"), b"vendored");
    std::fs::create_dir_all(project.join("work/vendor/.git")).unwrap();
    std::fs::create_dir_all(project.join("tasks/set")).unwrap();
    let policy = ScratchPolicy {
        repository: repository.clone(),
        project: project.clone(),
        task_root: project.join("tasks/set"),
        scratch_parent: base.join("scratch"),
        project_inputs: vec![PathBuf::from("work")],
        project_input_excludes: Vec::new(),
        combined: false,
        toolchain_path: "/usr/bin:/bin".into(),
        environment: BTreeMap::new(),
        environment_allowlist: Vec::new(),
        cargo_seed: None,
        timeout_secs: 60,
        output_bytes: 1 << 20,
        scratch_bytes: 1 << 30,
        build_cache: None,
    };
    Fixture {
        _temp: temp,
        project,
        repository,
        commit,
        policy,
    }
}

fn digest(f: &Fixture) -> String {
    data_digest_scoped(
        &f.project,
        &f.repository,
        Some(Path::new("tasks/set")),
        Path::new("work"),
        &[],
    )
    .unwrap()
}

#[cfg(unix)]
#[test]
fn scratch_copy_skips_the_nested_repository_the_digest_skips() {
    let f = fixture();
    let mut roots = ScratchRoots::prepare(&f.policy, &f.commit).unwrap();
    let copied = files(&roots.project().join("work"));
    roots.cleanup().unwrap();
    // The run's repository comes from its commit (the scratch worktree),
    // never from the live working tree inside the project input.
    assert!(
        !copied.iter().any(|path| path.starts_with("repo/")),
        "the scratch copy holds the nested repository's live tree: {copied:?}"
    );
    assert!(copied.contains("data.txt"), "{copied:?}");
    // A nested repository that is not the run's is project data: copied.
    assert!(copied.contains("vendor/lib.txt"), "{copied:?}");

    // The digest agrees: the skipped tree never moves the key, a copied
    // file always does.
    let before = digest(&f);
    write(&f.repository.join("live-only.txt"), b"edited working tree");
    write(&f.repository.join("tracked.txt"), b"edited tracked file");
    assert_eq!(before, digest(&f), "the nested repository moved the key");
    write(&f.project.join("work/vendor/lib.txt"), b"edited vendored");
    let vendored = digest(&f);
    assert_ne!(
        before, vendored,
        "a copied vendored file did not move the key"
    );
    write(&f.project.join("work/data.txt"), b"edited data");
    assert_ne!(vendored, digest(&f), "a copied file did not move the key");
}

#[test]
fn hermetic_copy_and_digest_share_the_nested_repository_rule() {
    let f = fixture();
    let destination = tempfile::tempdir().unwrap();
    let mut remaining = u64::MAX;
    copy_data(
        &f.project,
        Path::new(""),
        destination.path(),
        &f.repository,
        None,
        &mut remaining,
    )
    .unwrap();
    let copied = files(destination.path());
    assert!(!copied.iter().any(|path| path.starts_with("work/repo/")));
    assert!(copied.contains("work/data.txt"), "{copied:?}");
    assert!(copied.contains("work/vendor/lib.txt"), "{copied:?}");
}

#[test]
fn a_repository_at_the_project_root_skips_only_the_whole_walk() {
    let temp = tempfile::tempdir().unwrap();
    let root = archon_shell::paths::canonicalize(temp.path()).unwrap();
    // The repository is the project: only a walk of the whole project (the
    // empty path) is skipped, as before; a named input is still walked.
    let same = archon_workflow::acceptance_scratch::nested_repository(&root, &root);
    assert_eq!(same, Some(PathBuf::new()));
    assert!(nested_repository_skipped(Path::new(""), same.as_deref()));
    assert!(!nested_repository_skipped(
        Path::new("data"),
        same.as_deref()
    ));
    assert_eq!(
        archon_workflow::acceptance_scratch::nested_repository(&root.join("project"), &root),
        None,
        "a repository that holds the project is not nested in it"
    );
    assert_eq!(
        archon_workflow::acceptance_scratch::nested_repository(&root, &root.join("a/repo")),
        Some(PathBuf::from("a/repo"))
    );
}

#[cfg(unix)]
#[test]
fn a_symlink_to_the_repository_is_never_followed() {
    let f = fixture();
    std::os::unix::fs::symlink(&f.repository, f.project.join("work/alias")).unwrap();
    let mut roots = ScratchRoots::prepare(&f.policy, &f.commit).unwrap();
    let copied = files(&roots.project().join("work"));
    roots.cleanup().unwrap();
    assert!(
        !copied.iter().any(|path| path.starts_with("alias/")),
        "{copied:?}"
    );
    let before = digest(&f);
    write(
        &f.repository.join("tracked.txt"),
        b"edited through the alias",
    );
    assert_eq!(before, digest(&f));
}
