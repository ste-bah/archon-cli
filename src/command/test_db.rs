//! Self-deleting on-disk Cozo stores for tests.
//!
//! ## Why this exists (issue #156)
//!
//! Test fixtures in this crate used to open their sqlite store at a
//! `format!("/tmp/…-{uuid}.db")` path. On Windows `/tmp` is not a temp
//! directory — it is a *relative-to-drive-root* path, so the store landed in
//! `F:\tmp\` (or whichever drive the process was launched from). Nothing ever
//! deleted it. Measured on 2026-08-09: 32,758 orphaned files, 0.6 GB, over
//! eight days — the `.db` files plus the `.archon-cozo-write.lock` sibling
//! that `archon_cozo` creates next to every guarded store.
//!
//! [`TestDb`] owns a [`tempfile::TempDir`], which puts the files in the
//! platform's real temp directory and removes them when the guard drops.
//! Because the guard also owns the handle, the directory cannot be removed
//! while the database is still open — the ordering bug you get from returning
//! a bare `TempDir` that drops at the end of the fixture function.
//!
//! Tests that need no on-disk persistence at all should prefer an in-memory
//! instance (`DbInstance::new("mem", "", "")`) instead of this module.

use std::sync::Arc;

use cozo::DbInstance;

/// A database handle plus the temp directory holding its files.
///
/// `Deref` keeps call sites that only borrow the handle (`&db`,
/// `db.run_script(..)`) reading exactly as they did before. The guard just
/// has to stay bound for the test's duration, which `let db = test_db();`
/// already does.
pub struct TestDb<D> {
    db: D,
    _dir: tempfile::TempDir,
}

impl<D> TestDb<D> {
    /// Open a store in a fresh temp directory. `open` receives the full path
    /// the database should be created at.
    pub fn open(name: &str, open: impl FnOnce(&std::path::Path) -> D) -> Self {
        let dir = tempfile::tempdir().expect("create test temp dir");
        let path = dir.path().join(format!("{name}.db"));
        let db = open(&path);
        Self { db, _dir: dir }
    }
}

impl<D> std::ops::Deref for TestDb<D> {
    type Target = D;

    fn deref(&self) -> &Self::Target {
        &self.db
    }
}

impl TestDb<Arc<DbInstance>> {
    /// Clone the shared handle for call sites that must move an `Arc` into a
    /// longer-lived value (a `CommandContext`, a writer task). The `TestDb`
    /// must still outlive that clone, or the backing files vanish under it.
    pub fn arc(&self) -> Arc<DbInstance> {
        Arc::clone(&self.db)
    }
}

/// A guard-registered learning store with the learning schema applied.
///
/// This is the shape `crate::command::test_support::registered_learning_test_db`
/// hands out.
pub fn learning_test_db(name: &str) -> TestDb<Arc<DbInstance>> {
    let db = TestDb::open(name, |path| {
        archon_learning::cozo_guard::open_sqlite_guarded(
            &path.to_string_lossy(),
            "open test learning store",
        )
        .expect("open test learning store")
    });
    archon_learning::schema::ensure_learning_schema(&db).expect("ensure learning schema");
    db
}

/// A guard-registered store with no schema applied.
pub fn guarded_test_db(name: &str, context: &str) -> TestDb<Arc<DbInstance>> {
    TestDb::open(name, |path| {
        archon_cozo::open_sqlite_guarded_instance(
            &path.to_string_lossy(),
            context,
            archon_cozo::CozoGuardConfig::for_db_path(path),
        )
        .expect("open guarded test store")
        .db_arc()
    })
}
