use super::*;

#[test]
fn raw_operations_preserve_failure_status_while_checked_operations_refuse_it() {
    let root = tempfile::tempdir().unwrap();
    let isolated = IsolatedGit::new(root.path()).unwrap();
    let args = [std::ffi::OsStr::new("--no-such-isolated-operation")];
    let raw = isolated.raw_output(&args).unwrap();
    assert!(!raw.status.success());
    let checked = isolated.output(&args).unwrap_err();
    assert!(matches!(checked, CopyError::Operation(_)));
    let direct = isolated.command().args(args).output().unwrap();
    assert_eq!(direct.status.code(), raw.status.code());
    assert_eq!(direct.stderr, raw.stderr);
}

#[test]
fn raw_and_checked_operations_return_the_same_successful_output() {
    let root = tempfile::tempdir().unwrap();
    let isolated = IsolatedGit::new(root.path()).unwrap();
    let args = [std::ffi::OsStr::new("--version")];
    let raw = isolated.raw_output(&args).unwrap();
    assert!(raw.status.success());
    assert_eq!(isolated.output(&args).unwrap(), raw.stdout);
    isolated.run(&args).unwrap();
}
