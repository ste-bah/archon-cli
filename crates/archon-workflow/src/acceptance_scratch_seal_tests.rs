//! `TreeSeal` on a real filesystem: the digest always equals a full
//! inventory's, and only files whose metadata moved are read again.
use super::*;
use std::time::Duration;

fn full(root: &Path) -> String {
    crate::task_set_contract::content_digest(
        &serde_json::to_vec(&inventory(root).unwrap()).unwrap(),
    )
}

fn tree() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("registry/cache")).unwrap();
    for n in 0..20 {
        std::fs::write(
            dir.path().join(format!("registry/cache/crate-{n}.crate")),
            format!("bytes of crate {n}"),
        )
        .unwrap();
    }
    std::os::unix::fs::symlink("registry", dir.path().join("link")).unwrap();
    dir
}

/// A seal that trusts any digest taken after the file's last change, so a
/// test need not wait out the racy window.
fn seal() -> TreeSeal {
    TreeSeal::with_racy(Duration::ZERO)
}

#[test]
fn an_unchanged_tree_is_verified_without_reading_a_file_again() {
    let dir = tree();
    let mut seal = seal();
    let first = seal.digest(dir.path()).unwrap();
    assert_eq!(
        first,
        full(dir.path()),
        "the seal digests what inventory digests"
    );
    assert_eq!(seal.hashed(), 20, "the first walk reads every file once");
    let second = seal.digest(dir.path()).unwrap();
    assert_eq!(second, first);
    assert_eq!(seal.hashed(), 20, "no file was read again");
}

#[test]
fn a_modified_file_is_detected_and_only_it_is_read_again() {
    let dir = tree();
    let mut seal = seal();
    let before = seal.digest(dir.path()).unwrap();
    // Same length, and the old modification time put back: only the
    // status-change time records the write.
    let path = dir.path().join("registry/cache/crate-3.crate");
    let meta = std::fs::metadata(&path).unwrap();
    let original = Stat::of(&meta).unwrap();
    let mtime = meta.modified().unwrap();
    std::fs::write(&path, "BYTES OF CRATE 3").unwrap();
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(mtime)
        .unwrap();
    let rewritten = Stat::of(&std::fs::metadata(&path).unwrap()).unwrap();
    assert_eq!(original.len, rewritten.len);
    assert_eq!(original.mtime, rewritten.mtime);
    assert_ne!(original.ctime, rewritten.ctime);
    let after = seal.digest(dir.path()).unwrap();
    assert_ne!(after, before, "the rewrite is visible");
    assert_eq!(after, full(dir.path()));
    assert_eq!(seal.hashed(), 21, "only the rewritten file was read again");
}

#[test]
fn added_and_removed_files_are_detected() {
    let dir = tree();
    let mut seal = seal();
    let before = seal.digest(dir.path()).unwrap();
    std::fs::write(dir.path().join("registry/cache/new.crate"), "new").unwrap();
    let added = seal.digest(dir.path()).unwrap();
    assert_ne!(added, before);
    assert_eq!(added, full(dir.path()));
    assert_eq!(seal.hashed(), 21, "only the new file was read");
    std::fs::remove_file(dir.path().join("registry/cache/crate-0.crate")).unwrap();
    let removed = seal.digest(dir.path()).unwrap();
    assert_ne!(removed, added);
    assert_eq!(removed, full(dir.path()));
    assert_eq!(seal.hashed(), 21, "a removal needs no read");
    std::fs::remove_file(dir.path().join("link")).unwrap();
    std::os::unix::fs::symlink("elsewhere", dir.path().join("link")).unwrap();
    assert_eq!(
        seal.digest(dir.path()).unwrap(),
        full(dir.path()),
        "a retargeted link"
    );
}

#[test]
fn a_file_changed_within_the_racy_window_is_read_again() {
    let dir = tree();
    // Set mtimes explicitly; set_modified also refreshes ctime. The clock
    // recorded by the seal is controlled below, so scheduling is irrelevant.
    for n in 0..20 {
        std::fs::File::open(dir.path().join(format!("registry/cache/crate-{n}.crate")))
            .unwrap()
            .set_modified(if n % 2 == 0 {
                SystemTime::UNIX_EPOCH + Duration::from_secs(100)
            } else {
                SystemTime::now() + Duration::from_secs(3600)
            })
            .unwrap();
    }
    let mut seal = TreeSeal::default();
    seal.digest(dir.path()).unwrap();
    for known in seal.known.values_mut() {
        known.clock = known.stat.changed_at().unwrap();
    }
    seal.digest(dir.path()).unwrap();
    assert_eq!(seal.hashed(), 40, "young files are always read again");
    for known in seal.known.values_mut() {
        known.clock = known.stat.changed_at().unwrap() + Duration::from_secs(10);
    }
    seal.digest(dir.path()).unwrap();
    assert_eq!(seal.hashed(), 40, "old files are trusted");
}

/// Issue 255's measurement: `ARCHON_SEAL_BENCH_DIR` names a tree shaped
/// like a seeded Cargo home. Prints a full inventory against a seal's
/// re-verification.
#[test]
#[ignore = "measurement; set ARCHON_SEAL_BENCH_DIR and run with --ignored --nocapture"]
fn measure_full_hash_against_metadata_verification() {
    let Some(root) = std::env::var_os("ARCHON_SEAL_BENCH_DIR") else {
        return;
    };
    let root = PathBuf::from(root);
    let started = std::time::Instant::now();
    let reference = full(&root);
    let full_secs = started.elapsed().as_secs_f64();
    let mut seal = TreeSeal::default();
    let started = std::time::Instant::now();
    assert_eq!(seal.digest(&root).unwrap(), reference);
    let seal_first = started.elapsed().as_secs_f64();
    let started = std::time::Instant::now();
    assert_eq!(seal.digest(&root).unwrap(), reference);
    let seal_again = started.elapsed().as_secs_f64();
    println!(
        "files={} full_inventory={full_secs:.2}s seal_first={seal_first:.2}s seal_verify={seal_again:.2}s",
        seal.known.len()
    );
}
