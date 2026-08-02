//! Deriving the sidecar write-lock path for a database file.
//!
//! The path is a pure function of the database path and must stay pure — in
//! particular it must not create the database's parent directory, because it is
//! called on databases that do not exist yet.
use super::*;

#[test]
fn write_lock_path_is_sibling_sidecar() {
    // A real temp directory rather than a hardcoded `/tmp`: on Windows that
    // resolves to `<current drive>\tmp`, which usually does not exist, so the
    // test's own `canonicalize` panicked before it asserted anything.
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("archon-data.db");
    let expected_parent = canonical_resource_path(temp.path()).unwrap();
    assert_eq!(
        write_lock_path_for_db(&path),
        expected_parent.join("archon-data.db.archon-cozo-write.lock")
    );
}

#[test]
fn deriving_write_lock_path_does_not_create_database_parent() {
    let temp = tempfile::tempdir().unwrap();
    let parent = temp.path().join("missing").join("nested");
    let db_path = parent.join("learning.db");

    let lock_path = write_lock_path_for_db(&db_path);

    // Built through the crate's own resolver, not raw `canonicalize`. On
    // Windows the latter yields a verbatim `\\?\` path, which the resolver
    // deliberately simplifies, so a raw expectation no longer matches.
    let expected_parent = canonical_resource_path(temp.path())
        .unwrap()
        .join("missing")
        .join("nested");
    assert!(!parent.exists(), "lock-path derivation created directories");
    assert_eq!(
        lock_path,
        expected_parent.join("learning.db.archon-cozo-write.lock")
    );
}
