//! Test-only fixtures shared by this crate's unit tests.
//!
//! ## Why this exists
//!
//! Every `mod tests` in this crate used to open its sqlite store at a
//! `format!("/tmp/…-{uuid}.db")` path. On Windows `/tmp` is not a temp
//! directory — it resolves against the *current drive root* — so each test
//! run left an orphaned `.db` (and, for guarded stores, its
//! `.archon-cozo-write.lock` sibling) in e.g. `F:\tmp\`, forever. Nothing
//! ever deleted them.
//!
//! [`TestDb`] owns a [`tempfile::TempDir`], so the store's files live in the
//! platform's real temp directory and are removed when the guard drops.
//! Because the guard also owns the handle, the directory cannot be deleted
//! while the database is still open.

use cozo::DbInstance;

/// A database handle plus the temp directory holding its files.
///
/// `Deref` means call sites that only borrow the handle (`&db`,
/// `db.run_script(..)`) read exactly as they did before; the guard just has
/// to stay bound, which `let db = test_db();` already does.
pub(crate) struct TestDb<D> {
    db: D,
    _dir: tempfile::TempDir,
}

impl<D> TestDb<D> {
    /// Open a store in a fresh temp directory. `open` receives the full path
    /// the database should be created at.
    pub(crate) fn open(name: &str, open: impl FnOnce(&std::path::Path) -> D) -> Self {
        let dir = tempfile::tempdir().expect("create completion test temp dir");
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

/// An unguarded sqlite store — the shape `mod tests` used most often.
pub(crate) fn sqlite_test_db(name: &str) -> TestDb<DbInstance> {
    TestDb::open(name, |path| {
        DbInstance::new("sqlite", path, "").expect("open sqlite test store")
    })
}
