//! #589: normalized root paths and their neighboring domain decisions.

use super::check::{self, Verdict};
use super::index::{EvidenceIndex, Resolved};
use super::tests::{record, repository};

fn with_index(check: impl FnOnce(&mut EvidenceIndex, &std::path::Path)) {
    let dir = tempfile::tempdir().unwrap();
    let base = repository(dir.path());
    let mut index = EvidenceIndex::load(&record(dir.path(), &base)).unwrap();
    check(&mut index, dir.path());
}

fn assert_root(index: &mut EvidenceIndex, spelling: &str) {
    let relative = index.relative(spelling).expect("inside the repository");
    assert_eq!(relative, "");
    assert_eq!(index.resolve(&relative), Resolved::Exists(String::new()));
    assert!(index.is_directory(&relative));
    assert!(matches!(check::path(index, &relative), Verdict::Exists(_)));
    // Root existence needs no task owner. Exact declarations do not inherit
    // child owners, and directories have no file lines.
    index.declare("src/new.rs", "TASK-X-001");
    assert!(index.declared_by(&relative).is_empty());
    assert_eq!(index.line_count(&relative), 0);
    assert_eq!(index.readings(&relative, &[]), vec![String::new()]);
}

#[test]
fn claims_absolute_repository_root_is_present_directory_without_refutation() {
    with_index(|index, root| assert_root(index, &root.display().to_string()));
}

#[test]
fn claims_dot_and_dot_slash_are_present_directories_without_refutation() {
    with_index(|index, _| {
        assert_root(index, ".");
        assert_root(index, "./");
    });
}

#[test]
fn claims_repository_root_trailing_slash_is_present_directory_without_refutation() {
    with_index(|index, root| assert_root(index, &format!("{}/", root.display())));
}

#[test]
fn claims_outside_absolute_path_remains_untestable() {
    with_index(|index, _| {
        let outside = "/";
        assert_eq!(
            archon_workflow::repository_record::normalize_relative(outside),
            ""
        );
        assert_eq!(index.relative(outside), None);
        assert!(matches!(check::outside(outside), Verdict::Untested(_)));
    });
}

#[test]
fn claims_real_missing_file_remains_refuted() {
    with_index(|index, _| {
        assert!(matches!(
            check::path(index, "src/missing.rs"),
            Verdict::Refuted(_)
        ));
        assert!(!index.is_directory("src/missing.rs"));
    });
}

#[test]
fn claims_real_existing_file_remains_present() {
    with_index(|index, _| {
        assert!(matches!(
            check::path(index, "src/lib.rs"),
            Verdict::Exists(_)
        ));
        assert!(!index.is_directory("src/lib.rs"));
    });
}
