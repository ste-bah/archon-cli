use super::*;

fn vendor_scan(source: &str) {
    let root = tempfile::tempdir().unwrap();
    let path = root
        .path()
        .join("vendor/portable-pty/src/cmdbuilder_unix.rs");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, source).unwrap();
    let files = workspace_rust_files(root.path());
    assert!(
        files.contains(&path),
        "vendored child builder escaped the scan"
    );
    let found = source_violations("vendor/portable-pty/src/cmdbuilder_unix.rs", source);
    assert!(
        !found.is_empty(),
        "vendored spawn escaped the production detection pipeline"
    );
}

#[test]
fn vendor_direct_command_is_scanned() {
    vendor_scan("fn child() { std::process::Command::new(\"sh\"); }");
}
#[test]
fn vendor_renamed_command_is_scanned() {
    vendor_scan("use std::process::Command as C;\nfn child() { C::new(\"sh\"); }");
}
#[test]
fn vendor_raw_fork_is_scanned() {
    vendor_scan("fn child() { libc::fork(); }");
}
