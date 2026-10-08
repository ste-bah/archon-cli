use super::*;

fn remains_unchanged() {
    let names = ["PATH", "HOME", "ARCHON_274_ALLOWED_TOKEN"];
    let before: Vec<_> = names.iter().map(std::env::var_os).collect();
    catalog_digest_environment_child();
    let after: Vec<_> = names.iter().map(std::env::var_os).collect();
    assert_eq!(
        after, before,
        "direct ignored fixture mutated its caller's environment"
    );
}

#[test]
fn direct_ignored_fixture_preserves_existing_values() {
    if crate::test_environment::isolated() {
        return;
    }
    // An inherited marker for the fixture must not authorize mutation from
    // this other test, even though this monitor is itself isolated.
    unsafe {
        std::env::set_var(
            "ARCHON_ISOLATED_ENV_TEST",
            format!("{PREFIX}catalog_digest_environment_child"),
        );
    }
    remains_unchanged();
}
#[test]
fn direct_ignored_fixture_preserves_absent_values() {
    if crate::test_environment::isolated() {
        return;
    }
    for name in ["PATH", "HOME", "ARCHON_274_ALLOWED_TOKEN"] {
        unsafe {
            std::env::remove_var(name);
        }
    }
    remains_unchanged();
}
#[cfg(unix)]
#[test]
fn direct_ignored_fixture_preserves_non_unicode_values() {
    if crate::test_environment::isolated() {
        return;
    }
    use std::os::unix::ffi::OsStringExt;
    unsafe {
        std::env::set_var("PATH", std::ffi::OsString::from_vec(b"/path-\xff".to_vec()));
    }
    remains_unchanged();
}
