//! Issue 440: the scratch copy and its live audit skip the run's repository
//! nested inside a project input by the one rule the probe digest uses.
use super::*;

fn write(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

#[test]
fn copy_and_audit_skip_exactly_the_nested_repository() {
    let temp = tempfile::tempdir().unwrap();
    let project = archon_shell::paths::canonicalize(temp.path()).unwrap();
    write(&project.join("work/repo/src.rs"), b"live tree");
    write(
        &project.join("work/repo-notes.txt"),
        b"a sibling, not the repository",
    );
    write(&project.join("work/vendor/lib.rs"), b"vendored");
    let nested = nested_repository(&project, &project.join("work/repo"));
    assert_eq!(nested.as_deref(), Some(Path::new("work/repo")));

    let destination = temp.path().join("copy");
    let mut remaining = u64::MAX;
    copy_project(
        &project,
        Path::new("work"),
        &destination,
        (&[], nested.as_deref()),
        Path::new("tasks/set"),
        &mut remaining,
    )
    .unwrap();
    assert!(!destination.join("work/repo").exists());
    assert!(destination.join("work/repo-notes.txt").is_file());
    assert!(destination.join("work/vendor/lib.rs").is_file());

    let mut audited = BTreeMap::new();
    audit_path(
        &project,
        Path::new("work"),
        (&[], nested.as_deref()),
        None,
        &mut audited,
    )
    .unwrap();
    assert!(
        !audited
            .keys()
            .any(|key| key.starts_with("work/repo/") || key == "work/repo"),
        "{audited:?}"
    );
    assert!(audited.contains_key("work/repo-notes.txt"));
    assert!(audited.contains_key("work/vendor/lib.rs"));
}

#[test]
fn an_input_named_inside_the_repository_is_walked_by_every_walker_alike() {
    // The rule skips the repository's own path, met from above; an input
    // that names a path inside it is copied and audited (and digested).
    let temp = tempfile::tempdir().unwrap();
    let project = archon_shell::paths::canonicalize(temp.path()).unwrap();
    write(&project.join("repo/data/x.txt"), b"x");
    let nested = nested_repository(&project, &project.join("repo"));
    let destination = temp.path().join("copy");
    let mut remaining = u64::MAX;
    copy_project(
        &project,
        Path::new("repo/data"),
        &destination,
        (&[], nested.as_deref()),
        Path::new("tasks/set"),
        &mut remaining,
    )
    .unwrap();
    assert!(destination.join("repo/data/x.txt").is_file());
    let mut audited = BTreeMap::new();
    audit_path(
        &project,
        Path::new("repo/data"),
        (&[], nested.as_deref()),
        None,
        &mut audited,
    )
    .unwrap();
    assert!(audited.contains_key("repo/data/x.txt"));
}

#[test]
fn a_repository_outside_the_project_skips_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let base = archon_shell::paths::canonicalize(temp.path()).unwrap();
    assert_eq!(
        nested_repository(&base.join("project"), &base.join("repo")),
        None
    );
    assert_eq!(
        nested_repository(&base.join("repo/project"), &base.join("repo")),
        None
    );
    assert!(!nested_repository_skipped(Path::new("repo"), None));
}
