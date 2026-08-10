//! Test-only fixtures shared by this crate's unit tests.
//!
//! ## Why this exists
//!
//! Several `mod tests` in this crate used to open their Cozo store at a
//! `format!("/tmp/…-{uuid}.db")` path. On Windows `/tmp` is not a temp
//! directory — it resolves against the *current drive root* — so each test
//! run left an orphaned `.db` (and its `.archon-cozo-write.lock` sibling) in
//! e.g. `F:\tmp\`, forever. Nothing ever deleted them.
//!
//! [`TestDb`] owns a [`tempfile::TempDir`], so the store's files live in the
//! platform's real temp directory and are removed when the guard drops.
//! Because the guard also owns the handle, the directory cannot be deleted
//! while the database is still open.

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
        let dir = tempfile::tempdir().expect("create pipeline test temp dir");
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

/// A guard-registered sqlite store — the shape `mod tests` uses most often.
pub(crate) fn guarded_test_db(name: &str, context: &str) -> TestDb<archon_cozo::GuardedDbInstance> {
    TestDb::open(name, |path| {
        archon_cozo::open_sqlite_guarded_instance(
            &path.to_string_lossy(),
            context,
            archon_cozo::CozoGuardConfig::for_db_path(path),
        )
        .expect("open guarded sqlite test store")
    })
}
