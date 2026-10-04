use super::*;

fn repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    run_git(&["init", "-q"], dir.path()).unwrap();
    dir
}

#[test]
fn declared_ignored_deliverables_change_the_fingerprint() {
    let dir = repo();
    std::fs::write(dir.path().join(".gitignore"), "out.json\n").unwrap();
    let before = fingerprint(dir.path(), &["out.json".into()]);
    let undeclared = fingerprint(dir.path(), &[]);
    std::fs::write(dir.path().join("out.json"), "first").unwrap();
    assert_ne!(
        fingerprint(dir.path(), &["out.json".into()]),
        before,
        "declared ignored bytes count"
    );
    assert_eq!(
        fingerprint(dir.path(), &[]),
        undeclared,
        "undeclared ignored noise earns no credit"
    );
}

#[cfg(unix)]
#[test]
fn symlinks_are_never_followed_and_oversized_files_are_skipped() {
    use std::os::unix::fs::symlink;
    let dir = repo();
    let outside = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(outside.path(), "first").unwrap();
    symlink(outside.path(), dir.path().join("link")).unwrap();
    let before = fingerprint(dir.path(), &["out.json".into()]);
    std::fs::write(outside.path(), "changed outside").unwrap();
    assert_eq!(
        fingerprint(dir.path(), &["out.json".into()]),
        before,
        "never read through a link, including FIFOs/devices"
    );
    let large = std::fs::File::create(dir.path().join("large")).unwrap();
    large.set_len(2 * 1024 * 1024).unwrap();
    let before = fingerprint(dir.path(), &[]);
    large.set_len(3 * 1024 * 1024).unwrap();
    assert_eq!(
        fingerprint(dir.path(), &[]),
        before,
        "oversized bytes are not read"
    );
    std::fs::remove_file(dir.path().join("link")).unwrap();
    symlink("/dev/zero", dir.path().join("link")).unwrap();
    assert_eq!(
        fingerprint(dir.path(), &[]),
        before,
        "devices behind symlinks are never opened"
    );
}
