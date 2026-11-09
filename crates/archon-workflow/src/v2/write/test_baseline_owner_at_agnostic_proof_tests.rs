//! Issue #631: assertions describe the correct language-neutral behavior.
use super::*;
#[path = "../../../../../tests/fixtures/lang_projects/support.rs"]
mod fixture;

#[test]
#[ignore = "#635: needs a language-aware id reader"]
fn r6_h3_python_failure_maps_to_file() {
    let root = fixture::fixture("python");
    assert_eq!(
        test_file_at(
            &root,
            None,
            "python3 -m unittest test_fail",
            "test_fail.Failing.test_add"
        ),
        Some("test_fail.py".into())
    );
}
