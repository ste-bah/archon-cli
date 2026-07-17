use std::path::{Path, PathBuf};

use super::orphan_gate::scan_orphan_references;

fn write_source(root: &Path, relative_path: &str, content: &[u8]) -> PathBuf {
    let path = root.join(relative_path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, content).unwrap();
    path
}

fn assert_python_reference(content: &[u8]) {
    let tmp = tempfile::tempdir().unwrap();
    let foo = write_source(tmp.path(), "src/foo.py", b"VALUE = 1\n");
    write_source(tmp.path(), "src/references.py", content);

    let result = scan_orphan_references(&[foo], tmp.path(), || {}, |_| {});

    assert!(result.errors.is_empty());
    assert_eq!(result.references[0].references, ["src/references.py"]);
}

#[test]
fn scanner_matches_python_direct_relative_imports() {
    assert_python_reference(b"from .foo import VALUE\n");
}

#[test]
fn scanner_matches_python_parent_relative_imports() {
    assert_python_reference(b"from ..foo import VALUE\n");
}

#[test]
fn scanner_matches_python_dotted_from_imports() {
    assert_python_reference(b"from package.foo import VALUE\n");
}

#[test]
fn scanner_matches_python_dotted_plain_imports() {
    assert_python_reference(b"import package.foo\n");
}

#[test]
fn scanner_matches_python_dotted_plain_import_later_in_comma_list() {
    assert_python_reference(b"import unrelated.module, package.foo\n");
}

#[test]
fn scanner_rejects_python_partial_dotted_module_segments() {
    let tmp = tempfile::tempdir().unwrap();
    let foo = write_source(tmp.path(), "src/foo.py", b"VALUE = 1\n");
    write_source(
        tmp.path(),
        "src/references.py",
        b"from .foobar import VALUE\nfrom ..foo_bar import VALUE\nimport package.foo-bar\n",
    );

    let result = scan_orphan_references(&[foo], tmp.path(), || {}, |_| {});

    assert!(result.errors.is_empty());
    assert!(result.references[0].references.is_empty());
}
