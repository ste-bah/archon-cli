use super::super::test_baseline_run_base::tests::{commit_files, head, world};
use super::*;
#[test]
fn judged_tree_membership_is_independent_of_live_files() {
    let temp = tempfile::tempdir().unwrap();
    let (root, base, _, _) = world(temp.path());
    assert_eq!(
        test_file_at(&root, Some(&base), "check", r"src\shared_tests.rs::case"),
        Some("src/shared_tests.rs".into())
    );
    for id in [
        "./src/shared_tests.rs::case",
        "src/./shared_tests.rs::case",
        "src//shared_tests.rs::case",
    ] {
        assert_eq!(
            test_file_at(&root, Some(&base), "check", id),
            Some("src/shared_tests.rs".into())
        );
    }
    std::fs::remove_file(root.join("src/shared_tests.rs")).unwrap();
    assert_eq!(
        test_file_at(&root, Some(&base), "check", "src/shared_tests.rs::case"),
        Some("src/shared_tests.rs".into())
    );
    commit_files(&root, &["src/later.py"], "later file");
    assert_eq!(
        test_file_at(&root, Some(&base), "check", "src/later.py::case"),
        None
    );
    assert_eq!(
        test_file_at(&root, Some(&head(&root)), "check", "src/later.py::case"),
        Some("src/later.py".into())
    );
}
#[test]
fn modules_directories_and_missing_files_have_no_invented_owner() {
    let temp = tempfile::tempdir().unwrap();
    let (root, base, _, _) = world(temp.path());
    for id in [
        "shared::tests::case",
        "src::case",
        "missing.py::case",
        "src/shared_tests.rs/::case",
        "../outside::case",
    ] {
        assert_eq!(test_file_at(&root, Some(&base), "check", id), None, "{id}");
    }
}
#[test]
fn unavailable_recorded_trees_are_typed_errors() {
    let temp = tempfile::tempdir().unwrap();
    let (root, _, _, _) = world(temp.path());
    assert!(matches!(
        resolve(&root, None, "src/shared_tests.rs"),
        Err(ResolutionError::NoCommit)
    ));
    assert_eq!(
        test_file_at(&root, None, "check", "src/shared_tests.rs::case"),
        None
    );
    for commit in [
        "unknown",
        "0000000000000000000000000000000000000000",
        "-invalid",
    ] {
        assert!(
            matches!(
                resolve(&root, Some(commit), "src/shared_tests.rs"),
                Err(ResolutionError::Tree { .. })
            ),
            "{commit}"
        );
    }
}
