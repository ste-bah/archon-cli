use super::*;

#[test]
fn an_oversized_sibling_copy_has_the_typed_check_fault() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    let copied = root.path().join("scratch/project");
    let sibling = root.path().join("neighbor/child");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(&sibling).unwrap();
    std::fs::create_dir_all(&copied).unwrap();
    std::fs::write(sibling.join("input"), "data").unwrap();
    let mut remaining = 0;

    let copied_result = copy_external(
        &project,
        &project,
        &copied,
        &copied,
        &[(
            "cat ../neighbor/child/input".into(),
            crate::task_set_contract::TrustedCwd::ProjectRoot,
        )],
        &mut remaining,
    )
    .unwrap();

    assert_eq!(
        copied_result
            .failures
            .get(&0)
            .map(|failure| failure.1.clone()),
        Some(Some(
            crate::acceptance_scratch::faults::FaultCause::SiblingTooLarge
        ))
    );
}

#[test]
fn an_unavailable_sibling_shape_has_the_typed_check_fault() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    let copied = root.path().join("scratch/project");
    let sibling = root.path().join("neighbor/child");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(&sibling).unwrap();
    std::fs::create_dir_all(&copied).unwrap();
    std::fs::write(sibling.join("input"), "data").unwrap();
    std::fs::write(root.path().join("scratch/neighbor"), "occupied").unwrap();
    let mut remaining = u64::MAX;

    let copied_result = copy_external(
        &project,
        &project,
        &copied,
        &copied,
        &[(
            "cat ../neighbor/child/input".into(),
            crate::task_set_contract::TrustedCwd::ProjectRoot,
        )],
        &mut remaining,
    )
    .unwrap();

    assert_eq!(
        copied_result
            .failures
            .get(&0)
            .map(|failure| failure.1.clone()),
        Some(Some(
            crate::acceptance_scratch::faults::FaultCause::SiblingShapeRefused
        )),
        "{:?}",
        copied_result.failures
    );
}
