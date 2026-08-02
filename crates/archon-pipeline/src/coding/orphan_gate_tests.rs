use std::cell::Cell;
use std::path::{Path, PathBuf};

use super::orphan_gate::{orphan_result, scan_orphan_references};

fn write_source(root: &Path, relative_path: &str, content: &[u8]) -> PathBuf {
    let path = root.join(relative_path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, content).unwrap();
    path
}

#[test]
fn scanner_walks_once_and_reads_each_eligible_source_once_for_multiple_candidates() {
    let tmp = tempfile::tempdir().unwrap();
    let foo = write_source(tmp.path(), "src/foo.rs", b"pub fn foo() {}\n");
    let bar = write_source(tmp.path(), "src/bar.rs", b"pub fn bar() {}\n");
    write_source(tmp.path(), "src/lib.rs", b"mod foo;\nmod bar;\n");

    let walks = Cell::new(0);
    let reads = Cell::new(0);
    let result = scan_orphan_references(
        &[foo, bar],
        tmp.path(),
        || walks.set(walks.get() + 1),
        |_| reads.set(reads.get() + 1),
    );

    assert!(
        result.errors.is_empty(),
        "scanner errors: {:?}",
        result.errors
    );
    assert_eq!(
        walks.get(),
        1,
        "multiple candidates must share one traversal"
    );
    assert_eq!(reads.get(), 3, "each eligible source must be read once");
}

#[test]
fn orphan_result_preserves_deterministic_per_file_evidence() {
    let tmp = tempfile::tempdir().unwrap();
    let zeta = write_source(tmp.path(), "src/zeta.rs", b"pub fn zeta() {}\n");
    let alpha = write_source(tmp.path(), "src/alpha.rs", b"pub fn alpha() {}\n");
    write_source(tmp.path(), "src/lib.rs", b"mod zeta;\n");

    let result = orphan_result(&[zeta, alpha], tmp.path(), "now".into());

    assert!(!result.gate_passed);
    assert_eq!(
        result.evidence,
        "ORPHAN: src/alpha.rs — zero references\nOK: src/zeta.rs — referenced by: src/lib.rs"
    );
    assert_eq!(result.failures.len(), 1);
    assert_eq!(result.failures[0].file.as_deref(), Some("src/alpha.rs"));
}

#[test]
fn scanner_does_not_match_foo_inside_foobar() {
    let tmp = tempfile::tempdir().unwrap();
    let foo = write_source(tmp.path(), "src/foo.rs", b"pub fn foo() {}\n");
    write_source(tmp.path(), "src/lib.rs", b"mod foobar;\n");

    let result = scan_orphan_references(&[foo], tmp.path(), || {}, |_| {});

    assert!(result.errors.is_empty());
    assert!(result.references[0].references.is_empty());
}

#[test]
fn scanner_matches_literal_punctuation_paths_without_prefix_matching() {
    let tmp = tempfile::tempdir().unwrap();
    let foo = write_source(tmp.path(), "src/foo.rs", b"pub fn foo() {}\n");
    let punctuation = write_source(tmp.path(), "src/a+b.js", b"export {};\n");
    write_source(
        tmp.path(),
        "src/references.js",
        b"import foobar from \"./foobar\";\nimport hyphenated from \"./foo-bar\";\nimport \"./a+b\";\n",
    );

    let result = scan_orphan_references(&[foo, punctuation], tmp.path(), || {}, |_| {});

    assert!(result.errors.is_empty());
    let references = result
        .references
        .iter()
        .map(|candidate| {
            (
                candidate.candidate.as_str(),
                candidate.references.as_slice(),
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    assert!(references["src/foo.rs"].is_empty());
    assert_eq!(references["src/a+b.js"], ["src/references.js"]);
}

#[test]
fn scanner_matches_supported_reference_forms() {
    let tmp = tempfile::tempdir().unwrap();
    let py = write_source(tmp.path(), "src/python_target.py", b"VALUE = 1\n");
    let js = write_source(tmp.path(), "src/javascript_target.js", b"export {};\n");
    let ts = write_source(tmp.path(), "src/typescript_target.ts", b"export {};\n");
    let go = write_source(tmp.path(), "src/go_target.go", b"package src\n");
    let rust = write_source(tmp.path(), "src/rust_target.rs", b"pub fn marker() {}\n");
    write_source(
        tmp.path(),
        "src/references.py",
        b"from python_target import VALUE\n",
    );
    write_source(
        tmp.path(),
        "src/references.ts",
        b"import {\n  value,\n} from \"./typescript_target\";\n",
    );
    write_source(
        tmp.path(),
        "src/references.js",
        b"import {\n  value,\n} from \"./javascript_target\";\nconst ts = require(\n  \"./typescript_target\",\n);\n",
    );
    write_source(
        tmp.path(),
        "src/references.go",
        b"import (\n  \"example.com/project/go_target\"\n)\n",
    );
    write_source(
        tmp.path(),
        "src/lib.rs",
        b"use crate::{\n    rust_target::marker,\n};\n",
    );

    let result = scan_orphan_references(&[py, js, ts, go, rust], tmp.path(), || {}, |_| {});

    assert!(
        result.errors.is_empty(),
        "scanner errors: {:?}",
        result.errors
    );
    let references = result
        .references
        .iter()
        .map(|candidate| {
            (
                candidate.candidate.as_str(),
                candidate.references.as_slice(),
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(references["src/python_target.py"], ["src/references.py"]);
    assert_eq!(
        references["src/javascript_target.js"],
        ["src/references.js"]
    );
    assert_eq!(
        references["src/typescript_target.ts"],
        ["src/references.js", "src/references.ts"]
    );
    assert_eq!(references["src/go_target.go"], ["src/references.go"]);
    assert_eq!(references["src/rust_target.rs"], ["src/lib.rs"]);
}

#[test]
fn scanner_normalizes_relative_candidate_paths_against_absolute_project_root() {
    let tmp = tempfile::tempdir().unwrap();
    write_source(tmp.path(), "src/foo.rs", b"mod foo;\npub fn foo() {}\n");
    write_source(tmp.path(), "src/lib.rs", b"mod foo;\n");
    let relative = PathBuf::from("src/./nested/../foo.rs");

    let result = scan_orphan_references(&[relative], tmp.path(), || {}, |_| {});

    assert!(result.errors.is_empty());
    assert_eq!(result.references[0].references, ["src/lib.rs"]);
}

#[test]
fn scanner_rejects_distinct_candidates_with_the_same_stem() {
    let tmp = tempfile::tempdir().unwrap();
    let first = write_source(tmp.path(), "src/foo.rs", b"pub fn foo() {}\n");
    let second = write_source(tmp.path(), "other/foo.rs", b"pub fn foo() {}\n");

    let walks = Cell::new(0);
    let reads = Cell::new(0);
    let result = scan_orphan_references(
        &[first, second],
        tmp.path(),
        || walks.set(walks.get() + 1),
        |_| reads.set(reads.get() + 1),
    );

    assert!(result.references.is_empty());
    assert_eq!(
        result.errors,
        ["Ambiguous candidate stem 'foo': other/foo.rs, src/foo.rs"]
    );
    assert_eq!(walks.get(), 0);
    assert_eq!(reads.get(), 0);
}

#[test]
fn scanner_rejects_cross_language_candidates_with_the_same_stem() {
    let tmp = tempfile::tempdir().unwrap();
    let rust = write_source(tmp.path(), "src/foo.rs", b"pub fn foo() {}\n");
    let python = write_source(tmp.path(), "src/foo.py", b"VALUE = 1\n");

    let result = scan_orphan_references(&[rust, python], tmp.path(), || {}, |_| {});

    assert!(result.references.is_empty());
    assert_eq!(
        result.errors,
        ["Ambiguous candidate stem 'foo': src/foo.py, src/foo.rs"]
    );
}

#[test]
fn scanner_deduplicates_equivalent_candidate_paths() {
    let tmp = tempfile::tempdir().unwrap();
    let foo = write_source(tmp.path(), "src/foo.rs", b"pub fn foo() {}\n");
    write_source(tmp.path(), "src/lib.rs", b"mod foo;\n");

    let result = scan_orphan_references(
        &[PathBuf::from("src/./foo.rs"), foo],
        tmp.path(),
        || {},
        |_| {},
    );

    assert!(result.errors.is_empty());
    assert_eq!(result.references.len(), 1);
    assert_eq!(result.references[0].references, ["src/lib.rs"]);
}

#[test]
fn scanner_normalizes_root_and_rejects_candidates_outside_it() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("root/child/..");
    let absolute = write_source(tmp.path(), "root/src/foo.rs", b"pub fn foo() {}\n");
    write_source(tmp.path(), "root/src/lib.rs", b"mod foo;\n");

    let normalized = scan_orphan_references(
        &[PathBuf::from("src/./foo.rs"), absolute],
        &root,
        || {},
        |_| {},
    );
    assert!(normalized.errors.is_empty());
    assert_eq!(normalized.references.len(), 1);
    assert_eq!(normalized.references[0].candidate, "src/foo.rs");

    let outside = scan_orphan_references(&[PathBuf::from("../outside.rs")], &root, || {}, |_| {});
    assert!(outside.references.is_empty());
    assert_eq!(
        outside.errors,
        ["Candidate path outside project root: ../outside.rs"]
    );
}

#[cfg(unix)]
#[test]
fn scanner_reads_file_symlinks_but_skips_directory_cycles() {
    use std::os::unix::fs::symlink;

    let tmp = tempfile::tempdir().unwrap();
    let candidate = write_source(tmp.path(), "src/foo.rs", b"pub fn foo() {}\n");
    write_source(tmp.path(), "src/real-reference.rs", b"mod foo;\n");
    symlink(
        "real-reference.rs",
        tmp.path().join("src/link-reference.rs"),
    )
    .unwrap();
    symlink(tmp.path().join("src"), tmp.path().join("src/cycle-link")).unwrap();

    let result = scan_orphan_references(&[candidate], tmp.path(), || {}, |_| {});

    assert!(
        result.errors.is_empty(),
        "scanner errors: {:?}",
        result.errors
    );
    assert_eq!(
        result.references[0].references,
        ["src/link-reference.rs", "src/real-reference.rs"]
    );
}

#[cfg(unix)]
#[test]
fn scanner_reports_dangling_source_symlinks() {
    use std::os::unix::fs::symlink;

    let tmp = tempfile::tempdir().unwrap();
    let candidate = write_source(tmp.path(), "src/foo.rs", b"pub fn foo() {}\n");
    symlink("missing.rs", tmp.path().join("src/dangling.rs")).unwrap();

    let result = scan_orphan_references(&[candidate], tmp.path(), || {}, |_| {});

    assert!(
        result.references.is_empty(),
        "refs={:?} errors={:?}",
        result
            .references
            .iter()
            .map(|r| &r.candidate)
            .collect::<Vec<_>>(),
        result.errors
    );
    assert_eq!(result.errors.len(), 1, "errors={:?}", result.errors);
    assert!(result.errors[0].contains("Unable to inspect source symlink target"));
}

#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn scanner_skips_directory_symlinks_and_non_utf8_hidden_directories() {
    use std::os::unix::ffi::OsStringExt;
    use std::os::unix::fs::symlink;

    let tmp = tempfile::tempdir().unwrap();
    let candidate = write_source(tmp.path(), "src/foo.rs", b"pub fn foo() {}\n");
    write_source(tmp.path(), "src/lib.rs", b"mod foo;\n");
    let external = tempfile::tempdir().unwrap();
    write_source(external.path(), "outside.rs", b"mod foo;\n");
    symlink(external.path(), tmp.path().join("src/external-link")).unwrap();
    symlink(tmp.path(), tmp.path().join("src/cycle-link")).unwrap();
    let hidden = tmp
        .path()
        .join("src")
        .join(std::ffi::OsString::from_vec(b".hidden-\xff".to_vec()));
    std::fs::create_dir_all(&hidden).unwrap();
    std::fs::write(hidden.join("reference.rs"), b"mod foo;\n").unwrap();

    let result = scan_orphan_references(&[candidate], tmp.path(), || {}, |_| {});

    assert!(
        result.errors.is_empty(),
        "scanner errors: {:?}",
        result.errors
    );
    assert_eq!(result.references[0].references, ["src/lib.rs"]);
}
#[test]
fn scanner_allows_one_new_file_to_reference_another() {
    let tmp = tempfile::tempdir().unwrap();
    let foo = write_source(tmp.path(), "src/foo.rs", b"mod bar;\npub fn foo() {}\n");
    let bar = write_source(tmp.path(), "src/bar.rs", b"pub fn bar() {}\n");
    write_source(tmp.path(), "src/lib.rs", b"mod foo;\n");

    let result = scan_orphan_references(&[foo, bar], tmp.path(), || {}, |_| {});

    assert!(result.errors.is_empty());
    let references = result
        .references
        .iter()
        .map(|candidate| {
            (
                candidate.candidate.as_str(),
                candidate.references.as_slice(),
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(references["src/foo.rs"], ["src/lib.rs"]);
    assert_eq!(references["src/bar.rs"], ["src/foo.rs"]);
}

#[test]
fn scanner_returns_deterministic_candidate_and_reference_order() {
    let tmp = tempfile::tempdir().unwrap();
    let zeta = write_source(tmp.path(), "src/zeta.rs", b"pub fn zeta() {}\n");
    let alpha = write_source(tmp.path(), "src/alpha.rs", b"pub fn alpha() {}\n");
    write_source(tmp.path(), "src/z_ref.rs", b"mod alpha;\nmod zeta;\n");
    write_source(tmp.path(), "src/a_ref.rs", b"mod alpha;\nmod zeta;\n");

    let result = scan_orphan_references(&[zeta, alpha], tmp.path(), || {}, |_| {});

    assert!(result.errors.is_empty());
    assert_eq!(
        result
            .references
            .iter()
            .map(|candidate| candidate.candidate.as_str())
            .collect::<Vec<_>>(),
        vec!["src/alpha.rs", "src/zeta.rs"]
    );
    assert_eq!(
        result.references[0].references,
        vec!["src/a_ref.rs", "src/z_ref.rs"]
    );
    assert_eq!(
        result.references[1].references,
        vec!["src/a_ref.rs", "src/z_ref.rs"]
    );
}

#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn scanner_reports_non_utf8_candidate_name() {
    use std::os::unix::ffi::OsStringExt;

    let tmp = tempfile::tempdir().unwrap();
    let candidate = tmp
        .path()
        .join("src")
        .join(std::ffi::OsString::from_vec(b"bad-\xff.rs".to_vec()));
    std::fs::create_dir_all(candidate.parent().unwrap()).unwrap();
    std::fs::write(&candidate, b"pub fn bad() {}\n").unwrap();

    let result = scan_orphan_references(&[candidate], tmp.path(), || {}, |_| {});

    assert!(result.references.is_empty());
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.contains("Non-UTF8 candidate file name"))
    );
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.contains("Non-UTF8 source path"))
    );
}

#[cfg(unix)]
#[test]
fn scanner_reports_non_utf8_source_content() {
    let tmp = tempfile::tempdir().unwrap();
    let candidate = write_source(tmp.path(), "src/foo.rs", b"pub fn foo() {}\n");
    write_source(tmp.path(), "src/lib.rs", b"mod foo;\xff\n");

    let result = scan_orphan_references(&[candidate], tmp.path(), || {}, |_| {});

    assert!(result.references.is_empty());
    assert_eq!(result.errors.len(), 1);
    assert!(result.errors[0].contains("Non-UTF8 source content"));
}

#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn scanner_reports_non_utf8_source_path() {
    use std::os::unix::ffi::OsStringExt;

    let tmp = tempfile::tempdir().unwrap();
    let candidate = write_source(tmp.path(), "src/foo.rs", b"pub fn foo() {}\n");
    let source = tmp
        .path()
        .join("src")
        .join(std::ffi::OsString::from_vec(b"bad-\xff.rs".to_vec()));
    std::fs::write(&source, b"mod foo;\n").unwrap();

    let result = scan_orphan_references(&[candidate], tmp.path(), || {}, |_| {});

    assert!(result.references.is_empty());
    assert_eq!(result.errors.len(), 1);
    assert!(result.errors[0].contains("Non-UTF8 source path"));
}

#[cfg(not(unix))]
#[test]
fn dot_prefixed_candidate_file_name_is_scanned_by_its_full_name() {
    // Was `scanner_reports_missing_candidate_stem`, asserting that `src/.rs`
    // yields "Candidate file stem is absent". That can never happen:
    // `Path::new(".rs").file_stem()` is `Some(".rs")` -- a name beginning with
    // a dot and containing no other dot is entirely stem -- so the candidate is
    // accepted with stem ".rs". `file_stem()` returns `None` only for a path
    // with no final component, which `candidate_pattern` cannot receive, so the
    // guard it aimed at is unreachable from here.
    //
    // The test is `cfg(not(unix))` and had therefore never executed once:
    // Windows CI was disabled for the entire period it existed. It now asserts
    // what the code actually does.
    let tmp = tempfile::tempdir().unwrap();
    let candidate = write_source(tmp.path(), "src/.rs", b"pub fn bad() {}\n");

    let result = scan_orphan_references(&[candidate], tmp.path(), || {}, |_| {});

    assert!(
        result.errors.is_empty(),
        "a dot-prefixed file name is a valid candidate: {:?}",
        result.errors
    );
    assert_eq!(
        result.references.len(),
        1,
        "exactly one candidate should be tracked: {:?}",
        result
            .references
            .iter()
            .map(|reference| &reference.candidate)
            .collect::<Vec<_>>()
    );
}
