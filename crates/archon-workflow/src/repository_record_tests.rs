use super::*;

pub(crate) fn git(repo: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .expect("git starts");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A repository with one commit holding `src/lib.rs` and `docs/guide.md`.
pub(crate) fn committed_repo() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path();
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::create_dir_all(repo.join("docs")).unwrap();
    std::fs::write(repo.join("src/lib.rs"), "pub fn f() {}\n").unwrap();
    std::fs::write(repo.join("docs/guide.md"), "# guide\n").unwrap();
    git(repo, &["init", "-q"]);
    git(repo, &["config", "user.email", "test@example.invalid"]);
    git(repo, &["config", "user.name", "test"]);
    git(repo, &["add", "."]);
    git(repo, &["commit", "-q", "-m", "initial"]);
    temp
}

fn record_for(repo: &Path) -> RepositoryRecordV1 {
    RepositoryRecordV1 {
        schema_version: REPOSITORY_RECORD_SCHEMA_VERSION,
        repository_root: repo.canonicalize().unwrap().display().to_string(),
        base_commit: git_head(repo).unwrap(),
        decomposition_run_id: "wf-test".into(),
        recorded_at: "2026-09-19T00:00:00Z".into(),
    }
}

#[test]
fn a_committed_repository_reports_its_head_and_a_fresh_one_is_unborn() {
    let repo = committed_repo();
    let head = git_head(repo.path()).unwrap();
    assert_eq!(head.len(), 40, "{head}");
    assert!(is_git_checkout(repo.path()));

    let fresh = tempfile::tempdir().unwrap();
    git(fresh.path(), &["init", "-q"]);
    assert_eq!(git_head(fresh.path()).unwrap(), UNBORN_BASE_COMMIT);
    assert!(is_git_checkout(fresh.path()));

    let plain = tempfile::tempdir().unwrap();
    assert!(!is_git_checkout(plain.path()));
    assert!(git_head(plain.path()).is_err());
}

#[test]
fn the_record_round_trips_and_an_absent_one_is_none() {
    let repo = committed_repo();
    let tasks = tempfile::tempdir().unwrap();
    assert_eq!(read_repository_record(tasks.path()).unwrap(), None);
    let record = record_for(repo.path());
    write_repository_record(tasks.path(), &record).unwrap();
    assert_eq!(read_repository_record(tasks.path()).unwrap(), Some(record));
    assert!(!tasks.path().join("repository.lock.tmp").exists());
}

#[test]
fn a_malformed_record_is_an_error_not_a_legacy_set() {
    let tasks = tempfile::tempdir().unwrap();
    std::fs::write(repository_record_path(tasks.path()), b"{not json").unwrap();
    let error = read_repository_record(tasks.path()).unwrap_err().to_string();
    assert!(error.contains("malformed"), "{error}");
}

#[test]
fn the_tree_at_the_base_commit_answers_existence_for_files_and_directories() {
    let repo = committed_repo();
    let tree = RepositoryTree::load(&record_for(repo.path())).unwrap();
    assert!(tree.exists_at_base("src/lib.rs"));
    assert!(tree.exists_at_base("./src//lib.rs"));
    assert!(tree.exists_at_base("src"));
    assert!(tree.is_dir_at_base("src"));
    assert!(!tree.is_dir_at_base("src/lib.rs"));
    assert!(!tree.exists_at_base("src/missing.rs"));
    assert!(tree.truth("src/lib.rs").certainly_exists());
    assert!(tree.truth("src/missing.rs").certainly_absent());
    // Present in the checkout but not at the base: neither certainty holds.
    std::fs::write(repo.path().join("src/new.rs"), "").unwrap();
    let truth = tree.truth("src/new.rs");
    assert!(!truth.certainly_exists() && !truth.certainly_absent());
    // Deleted from the checkout after the base: neither certainty holds.
    std::fs::remove_file(repo.path().join("docs/guide.md")).unwrap();
    let truth = tree.truth("docs/guide.md");
    assert!(!truth.certainly_exists() && !truth.certainly_absent());
}

#[test]
fn an_unborn_base_lists_nothing_and_absolute_paths_resolve_against_the_root() {
    let fresh = tempfile::tempdir().unwrap();
    git(fresh.path(), &["init", "-q"]);
    let tree = RepositoryTree::load(&record_for(fresh.path())).unwrap();
    assert!(tree.paths_at_base().is_empty());
    assert_eq!(tree.base_commit(), UNBORN_BASE_COMMIT);
    let inside = format!("{}/a/b.rs", tree.root().display());
    assert_eq!(tree.relative_to_root(&inside).as_deref(), Some("a/b.rs"));
    assert_eq!(tree.relative_to_root("/definitely/elsewhere/x.rs"), None);
    assert_eq!(tree.relative_to_root("./x/y.rs").as_deref(), Some("x/y.rs"));
}
