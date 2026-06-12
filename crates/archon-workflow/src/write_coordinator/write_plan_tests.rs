//! Unit tests for write_plan (child module via #[path]; file-size guard).

use super::*;
use serde_json::json;


fn root() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("src")).expect("mkdir src");
    std::fs::write(dir.path().join("src/lib.rs"), "// lib").expect("write");
    dir
}

#[test]
fn rejects_empty_and_nul() {
    let r = root();
    assert!(matches!(
        normalize_target("", r.path()),
        Err(WritePlanError::InvalidTargetPath(_))
    ));
    assert!(matches!(
        normalize_target("src/\0bad", r.path()),
        Err(WritePlanError::InvalidTargetPath(_))
    ));
}

#[test]
fn rejects_traversal() {
    let r = root();
    assert!(matches!(
        normalize_target("../etc/passwd", r.path()),
        Err(WritePlanError::TraversalEscape(_))
    ));
    assert!(matches!(
        normalize_target("src/../../x", r.path()),
        Err(WritePlanError::TraversalEscape(_))
    ));
    assert!(matches!(
        normalize_target("src\\..\\x", r.path()),
        Err(WritePlanError::TraversalEscape(_))
    ));
}

#[test]
fn rejects_absolute_outside_root() {
    let r = root();
    assert!(matches!(
        normalize_target("/absolute/outside", r.path()),
        Err(WritePlanError::AbsoluteEscape(_))
    ));
}

#[test]
fn accepts_absolute_inside_root() {
    let r = root();
    let abs = r.path().join("src/lib.rs");
    let n = normalize_target(abs.to_str().unwrap(), r.path()).expect("inside root ok");
    assert_eq!(n.as_str(), "src/lib.rs");
}

#[test]
fn rejects_empty_segment_and_trailing_slash() {
    let r = root();
    assert!(matches!(
        normalize_target("src//lib.rs", r.path()),
        Err(WritePlanError::EmptySegment(_))
    ));
    assert!(matches!(
        normalize_target("src/lib.rs/", r.path()),
        Err(WritePlanError::EmptySegment(_))
    ));
}

#[test]
fn three_os_forms_normalize_identically() {
    let r = root();
    for raw in ["src/lib.rs", "src\\lib.rs"] {
        let n = normalize_target(raw, r.path()).unwrap_or_else(|e| panic!("{raw}: {e}"));
        assert_eq!(n.as_str(), "src/lib.rs", "form: {raw}");
    }
    std::fs::create_dir_all(r.path().join("src/sub")).expect("mkdir");
    let mixed = normalize_target("src\\sub/new.rs", r.path()).expect("mixed form");
    assert_eq!(mixed.as_str(), "src/sub/new.rs");
}

#[test]
fn not_yet_existing_paths_are_fine() {
    let r = root();
    let n = normalize_target("brand/new/file.rs", r.path()).expect("future file ok");
    assert_eq!(n.as_str(), "brand/new/file.rs");
}

#[cfg(unix)]
#[test]
fn rejects_symlink_escape() {
    let r = root();
    let outside = tempfile::tempdir().expect("outside dir");
    std::os::unix::fs::symlink(outside.path(), r.path().join("link")).expect("symlink");
    assert!(matches!(
        normalize_target("link/file.txt", r.path()),
        Err(WritePlanError::SymlinkEscape(_))
    ));
}

#[cfg(unix)]
#[test]
fn rejects_symlink_escape_to_nonexistent_target() {
    // Sherlock Gate-3 regression: canonicalize fails on a missing target, and
    // a lexical starts_with would pass `root/../../escapee`. Must still reject.
    let r = root();
    std::os::unix::fs::symlink("../../escapee", r.path().join("link")).expect("symlink");
    assert!(matches!(
        normalize_target("link/file.txt", r.path()),
        Err(WritePlanError::SymlinkEscape(_))
    ));
}

#[cfg(unix)]
#[test]
fn allows_symlink_to_nonexistent_target_inside_root() {
    let r = root();
    std::os::unix::fs::symlink("future-dir", r.path().join("link")).expect("symlink");
    let n = normalize_target("link/file.txt", r.path()).expect("inside root ok");
    assert_eq!(n.as_str(), "future-dir/file.txt");
}

#[cfg(unix)]
#[test]
fn rejects_chained_symlink_escape_to_nonexistent_target() {
    // Sherlock Gate-3 V2: link1 -> link2 -> ../../escapee (nonexistent). The
    // lexical fallback could not see link2's target; full resolution must.
    let r = root();
    std::os::unix::fs::symlink("link2", r.path().join("link1")).expect("symlink");
    std::os::unix::fs::symlink("../../escapee", r.path().join("link2")).expect("symlink");
    assert!(matches!(
        normalize_target("link1/file.txt", r.path()),
        Err(WritePlanError::SymlinkEscape(_))
    ));
}

#[cfg(unix)]
#[test]
fn allows_chained_symlink_to_nonexistent_inside_root() {
    let r = root();
    std::os::unix::fs::symlink("link2", r.path().join("link1")).expect("symlink");
    std::os::unix::fs::symlink("future-inside", r.path().join("link2")).expect("symlink");
    let n = normalize_target("link1/file.txt", r.path()).expect("inside chain ok");
    assert_eq!(n.as_str(), "future-inside/file.txt");
}

#[cfg(unix)]
#[test]
fn symlink_loop_terminates_with_error() {
    let r = root();
    std::os::unix::fs::symlink("b", r.path().join("a")).expect("symlink");
    std::os::unix::fs::symlink("a", r.path().join("b")).expect("symlink");
    assert!(matches!(
        normalize_target("a/file.txt", r.path()),
        Err(WritePlanError::SymlinkEscape(_))
    ));
}

#[cfg(unix)]
#[test]
fn allows_symlink_staying_inside() {
    let r = root();
    std::os::unix::fs::symlink(r.path().join("src"), r.path().join("alias"))
        .expect("symlink");
    let n = normalize_target("alias/lib.rs", r.path()).expect("inside symlink ok");
    assert_eq!(n.as_str(), "src/lib.rs");
}

#[test]
fn case_fold_macos_like_but_not_linux() {
    assert_eq!(fold_case_for_os("Src/.Git/HEAD", "macos"), "src/.git/head");
    assert_eq!(fold_case_for_os("Src/.Git/HEAD", "windows"), "src/.git/head");
    assert_eq!(fold_case_for_os("Src/.Git/HEAD", "linux"), "Src/.Git/HEAD");
}

#[test]
fn provenance_item_wins() {
    let payload = json!({"target_files": ["a.rs"], "expected_target_files": ["b.rs"]});
    let (files, src) = resolve_target_files(&payload, &["c.rs".into()]).expect("ok");
    assert_eq!(files, vec!["a.rs"]);
    assert_eq!(src, TargetFilesSource::Item);
}

#[test]
fn provenance_item_expected_second() {
    let payload = json!({"expected_target_files": ["b.rs"]});
    let (files, src) = resolve_target_files(&payload, &["c.rs".into()]).expect("ok");
    assert_eq!(files, vec!["b.rs"]);
    assert_eq!(src, TargetFilesSource::ItemExpected);
}

#[test]
fn provenance_stage_level_last() {
    let payload = json!({"name": "no targets here"});
    let (files, src) = resolve_target_files(&payload, &["c.rs".into()]).expect("ok");
    assert_eq!(files, vec!["c.rs"]);
    assert_eq!(src, TargetFilesSource::StageLevel);
}

#[test]
fn provenance_missing_everywhere_errs() {
    let payload = json!({"name": "nothing"});
    assert!(matches!(
        resolve_target_files(&payload, &[]),
        Err(WritePlanError::MissingTargets)
    ));
}

#[test]
fn non_string_target_entries_err() {
    let payload = json!({"target_files": [42, true]});
    assert!(matches!(
        resolve_target_files(&payload, &[]),
        Err(WritePlanError::InvalidTargetPath(_))
    ));
}

#[test]
fn keys_conflict_file_dir_matrix() {
    use ResourceKey::*;
    assert!(keys_conflict(&File("a/b".into()), &File("a/b".into())));
    assert!(!keys_conflict(&File("a/b".into()), &File("a/c".into())));
    assert!(keys_conflict(&File("a/b".into()), &Dir("a".into())));
    assert!(!keys_conflict(&File("a/b".into()), &Dir("c".into())));
    assert!(!keys_conflict(&File("ab/x".into()), &Dir("a".into())), "no prefix false-positive");
    assert!(keys_conflict(&Dir("a/b".into()), &Dir("a".into())));
    assert!(!keys_conflict(&Dir("a".into()), &Dir("b".into())));
}

#[test]
fn keys_conflict_glob_matrix() {
    use ResourceKey::*;
    assert!(keys_conflict(
        &Glob("crates/foo/src/*".into()),
        &File("crates/foo/src/lib.rs".into())
    ));
    assert!(!keys_conflict(
        &Glob("crates/foo/src/*".into()),
        &File("crates/bar/src/lib.rs".into())
    ));
    assert!(keys_conflict(&Glob("src/*".into()), &Dir("src".into())));
    assert!(keys_conflict(&Glob("src/a*".into()), &Glob("src/ab*".into())));
    assert!(!keys_conflict(&Glob("src/a*".into()), &Glob("docs/b*".into())));
}

#[test]
fn resource_key_ord_file_dir_glob() {
    use ResourceKey::*;
    let set: BTreeSet<ResourceKey> = [Glob("a".into()), Dir("a".into()), File("a".into())]
        .into_iter()
        .collect();
    let order: Vec<ResourceKey> = set.into_iter().collect();
    assert_eq!(
        order,
        vec![File("a".into()), Dir("a".into()), Glob("a".into())]
    );
}

#[test]
fn resource_keys_include_created_parent_dirs_only() {
    let r = root();
    let existing = normalize_target("src/other.rs", r.path()).expect("ok");
    let fresh = normalize_target("new/sub/file.rs", r.path()).expect("ok");
    let keys =
        resource_keys_for_targets(&[existing, fresh], r.path(), &[]).expect("keys ok");
    assert!(keys.contains(&ResourceKey::File("src/other.rs".into())));
    assert!(keys.contains(&ResourceKey::File("new/sub/file.rs".into())));
    assert!(keys.contains(&ResourceKey::Dir("new".into())));
    assert!(keys.contains(&ResourceKey::Dir("new/sub".into())));
    assert!(
        !keys.contains(&ResourceKey::Dir("src".into())),
        "existing parent dirs must not produce dir keys"
    );
}

#[test]
fn declared_globs_become_glob_keys_and_malformed_errs() {
    let r = root();
    let keys = resource_keys_for_targets(&[], r.path(), &["src/gen_*.rs".into()])
        .expect("glob ok");
    assert!(keys.contains(&ResourceKey::Glob("src/gen_*.rs".into())));
    assert!(matches!(
        resource_keys_for_targets(&[], r.path(), &["src/[bad".into()]),
        Err(WritePlanError::MalformedGlob(_))
    ));
}

#[test]
fn baseline_id_prefixes() {
    assert_eq!(parse_baseline_id("blake3:abc").expect("ok"), "blake3:abc");
    assert_eq!(parse_baseline_id("git:sha").expect("ok"), "git:sha");
    assert!(matches!(
        parse_baseline_id("abc"),
        Err(WritePlanError::InvalidBaselineId(_))
    ));
}

