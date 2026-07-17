use std::path::{Path, PathBuf};

use super::orphan_gate::scan_orphan_references;

fn write_source(root: &Path, relative_path: &str, content: &[u8]) -> PathBuf {
    let path = root.join(relative_path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, content).unwrap();
    path
}

#[test]
fn scanner_matches_single_aliased_go_import() {
    let tmp = tempfile::tempdir().unwrap();
    let candidate = write_source(tmp.path(), "src/go_target.go", b"package src\n");
    write_source(
        tmp.path(),
        "src/references.go",
        b"import target \"example.com/project/go_target\"\n",
    );

    let result = scan_orphan_references(&[candidate], tmp.path(), || {}, |_| {});

    assert!(result.errors.is_empty());
    assert_eq!(result.references[0].references, ["src/references.go"]);
}

#[test]
fn scanner_matches_grouped_aliased_go_import_after_other_entries() {
    let tmp = tempfile::tempdir().unwrap();
    let candidate = write_source(tmp.path(), "src/go_target.go", b"package src\n");
    write_source(
        tmp.path(),
        "src/references.go",
        b"import (\n  \"example.com/project/other\"\n  target \"example.com/project/go_target\"\n)\n",
    );

    let result = scan_orphan_references(&[candidate], tmp.path(), || {}, |_| {});

    assert!(result.errors.is_empty());
    assert_eq!(result.references[0].references, ["src/references.go"]);
}

#[test]
fn scanner_matches_later_bare_grouped_go_import() {
    let tmp = tempfile::tempdir().unwrap();
    let candidate = write_source(tmp.path(), "src/go_target.go", b"package src\n");
    write_source(
        tmp.path(),
        "src/references.go",
        b"import (\n  \"example.com/project/other\"\n  \"example.com/project/go_target\"\n)\n",
    );

    let result = scan_orphan_references(&[candidate], tmp.path(), || {}, |_| {});

    assert!(result.errors.is_empty());
    assert_eq!(result.references[0].references, ["src/references.go"]);
}

#[test]
fn scanner_matches_blank_and_dot_go_imports() {
    let tmp = tempfile::tempdir().unwrap();
    let blank = write_source(tmp.path(), "src/blank_target.go", b"package src\n");
    let dot = write_source(tmp.path(), "src/dot_target.go", b"package src\n");
    write_source(
        tmp.path(),
        "src/references.go",
        b"import _ \"example.com/project/blank_target\"\nimport . \"example.com/project/dot_target\"\n",
    );

    let result = scan_orphan_references(&[blank, dot], tmp.path(), || {}, |_| {});

    assert!(result.errors.is_empty());
    assert_eq!(result.references[0].references, ["src/references.go"]);
    assert_eq!(result.references[1].references, ["src/references.go"]);
}
