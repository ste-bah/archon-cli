use super::*;

#[test]
fn a_sibling_root_named_target_is_copied_with_build_cache() {
    // The root is copied; only target directories below it are omitted.
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    let copied = root.path().join("scratch/project");
    let sibling = root.path().join("target");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(&copied).unwrap();
    std::fs::create_dir_all(sibling.join("target/debug")).unwrap();
    std::fs::write(sibling.join("needed.txt"), "current target root").unwrap();
    std::fs::write(sibling.join("target/debug/output"), "nested build").unwrap();
    let mut remaining = u64::MAX;
    copied_all(copy_external(
        &project,
        &project,
        &copied,
        &copied,
        &[(
            "cat ../target/needed.txt".into(),
            crate::task_set_contract::TrustedCwd::ProjectRoot,
        )],
        &mut remaining,
        true,
    ));
    let copied_target = copied.parent().unwrap().join("target");
    assert_eq!(
        std::fs::read_to_string(copied_target.join("needed.txt")).unwrap(),
        "current target root"
    );
    assert!(!copied_target.join("target").exists());
}

#[test]
fn a_sibling_root_named_target_is_refused_without_build_cache() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    let copied = root.path().join("scratch/project");
    let sibling = root.path().join("target");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(&copied).unwrap();
    std::fs::create_dir_all(&sibling).unwrap();
    std::fs::write(sibling.join("item"), "sibling").unwrap();
    let mut remaining = u64::MAX;
    let result = copy_external(
        &project,
        &project,
        &copied,
        &copied,
        &[(
            "test -f ../target/item".into(),
            crate::task_set_contract::TrustedCwd::ProjectRoot,
        )],
        &mut remaining,
        false,
    )
    .unwrap();
    assert_eq!(result.failures.len(), 1);
    assert!(!copied.parent().unwrap().join("target").exists());
}

#[test]
fn a_site_internal_sibling_is_refused_before_copy_with_and_without_cache() {
    for build_cache in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("project");
        let copied = root.path().join("scratch/project");
        let sibling = root.path().join("project-baseline");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&copied).unwrap();
        std::fs::create_dir_all(&sibling).unwrap();
        std::fs::write(sibling.join("item"), "sibling").unwrap();
        let mut remaining = u64::MAX;
        let result = copy_external(
            &project,
            &project,
            &copied,
            &copied,
            &[(
                "test -f ../project-baseline/item".into(),
                crate::task_set_contract::TrustedCwd::ProjectRoot,
            )],
            &mut remaining,
            build_cache,
        )
        .unwrap();
        assert_eq!(result.failures.len(), 1, "build_cache={build_cache}");
        assert!(!copied.parent().unwrap().join("project-baseline").exists());
    }
}

#[test]
fn a_repo_alias_refusal_has_a_cause_matching_its_author_text() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("workspace");
    let repository = root.path().join("repository");
    let copied_project = root.path().join("scratch/project");
    let copied_repository = root.path().join("scratch/repo");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(&repository).unwrap();
    std::fs::create_dir_all(&copied_project).unwrap();
    std::fs::create_dir_all(&copied_repository).unwrap();
    let mut remaining = u64::MAX;
    let copied = copy_external(
        &project,
        &repository,
        &copied_project,
        &copied_repository,
        &[(
            "test -f ../repo/item".into(),
            crate::task_set_contract::TrustedCwd::RepoRoot,
        )],
        &mut remaining,
        false,
    )
    .unwrap();
    let cause = copied.failures[&0].1.as_ref().unwrap();
    let fault = cause.fault();
    let text = format!("{fault:?}");
    assert!(text.contains("leaves its selected root"), "{text}");
    assert!(text.contains("site uses for its own copy"), "{text}");
    assert!(!text.contains("resolves to the project root"), "{text}");
}
