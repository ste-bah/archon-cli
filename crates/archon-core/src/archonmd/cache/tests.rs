//! Issue #171 Part 5 — revalidation must notice content changes *and*
//! files appearing or disappearing from the hierarchy.

use super::*;
use tempfile::TempDir;

/// A working dir whose own `ARCHON.md` we control. Ancestors and the global
/// file may also contribute sections on a developer machine, so assertions are
/// on deltas and on substrings, never on absolute file counts.
fn workdir() -> TempDir {
    TempDir::new().unwrap()
}

fn write_archon_md(dir: &Path, body: &str) {
    std::fs::write(dir.join("ARCHON.md"), body).unwrap();
}

#[test]
fn repeated_loads_read_the_files_once() {
    let tmp = workdir();
    write_archon_md(tmp.path(), "project rules v1");
    let cache = ArchonMdCache::new();

    let first = cache.load(tmp.path());
    let after_first = cache.stats();
    assert_eq!(after_first.misses, 1);
    assert_eq!(after_first.hits, 0);
    assert!(first.contains("project rules v1"));

    for _ in 0..4 {
        let again = cache.load(tmp.path());
        assert_eq!(&*again, &*first);
    }

    let stats = cache.stats();
    assert_eq!(stats.misses, 1, "hierarchy must be read exactly once");
    assert_eq!(stats.hits, 4);
    assert_eq!(
        stats.files_read, after_first.files_read,
        "no further file reads after the cold load"
    );
}

#[test]
fn cached_render_matches_the_uncached_loader() {
    let tmp = workdir();
    write_archon_md(tmp.path(), "identical output check");
    let cache = ArchonMdCache::new();

    let direct = super::super::load_hierarchical_archon_md(tmp.path());
    let cached = cache.load(tmp.path());
    assert_eq!(&*cached, direct.as_str());

    // And on the hit path too.
    let cached_again = cache.load(tmp.path());
    assert_eq!(&*cached_again, direct.as_str());
}

#[test]
fn content_change_invalidates_the_entry() {
    let tmp = workdir();
    write_archon_md(tmp.path(), "v1 body");
    let cache = ArchonMdCache::new();
    assert!(cache.load(tmp.path()).contains("v1 body"));

    // Different length, so the stamp differs regardless of mtime granularity.
    write_archon_md(tmp.path(), "v2 body, materially longer than the first");
    let reloaded = cache.load(tmp.path());
    assert!(reloaded.contains("v2 body"));
    assert!(!reloaded.contains("v1 body"));
    assert_eq!(cache.stats().misses, 2);
}

#[test]
fn a_file_added_to_the_hierarchy_invalidates_the_entry() {
    let tmp = workdir();
    write_archon_md(tmp.path(), "root level rules");
    let cache = ArchonMdCache::new();
    let before = cache.load(tmp.path());
    assert!(!before.contains("preferred dot-archon rules"));

    // `.archon/ARCHON.md` outranks the plain file: discovery now resolves to a
    // different path, which no mtime comparison on the old path would catch.
    std::fs::create_dir_all(tmp.path().join(".archon")).unwrap();
    std::fs::write(
        tmp.path().join(".archon").join("ARCHON.md"),
        "preferred dot-archon rules",
    )
    .unwrap();

    let after = cache.load(tmp.path());
    assert!(after.contains("preferred dot-archon rules"));
    assert!(!after.contains("root level rules"));
    assert_eq!(cache.stats().misses, 2);
}

#[test]
fn a_file_removed_from_the_hierarchy_invalidates_the_entry() {
    let tmp = workdir();
    write_archon_md(tmp.path(), "will be deleted");
    let cache = ArchonMdCache::new();
    assert!(cache.load(tmp.path()).contains("will be deleted"));

    std::fs::remove_file(tmp.path().join("ARCHON.md")).unwrap();
    let after = cache.load(tmp.path());
    assert!(!after.contains("will be deleted"));
    assert_eq!(cache.stats().misses, 2);
}

#[test]
fn distinct_working_dirs_get_distinct_entries() {
    let a = workdir();
    let b = workdir();
    write_archon_md(a.path(), "alpha rules");
    write_archon_md(b.path(), "beta rules");
    let cache = ArchonMdCache::new();

    assert!(cache.load(a.path()).contains("alpha rules"));
    assert!(cache.load(b.path()).contains("beta rules"));
    assert_eq!(cache.stats().misses, 2);

    assert!(cache.load(a.path()).contains("alpha rules"));
    assert!(cache.load(b.path()).contains("beta rules"));
    assert_eq!(cache.stats().hits, 2);
    assert_eq!(cache.stats().misses, 2);
}

#[test]
fn a_working_dir_with_no_instructions_files_is_still_cached() {
    let tmp = workdir();
    let cache = ArchonMdCache::new();
    let first = cache.load(tmp.path());
    let second = cache.load(tmp.path());
    assert_eq!(&*first, &*second);
    assert_eq!(cache.stats().hits, 1);
    assert_eq!(cache.stats().misses, 1);
}
