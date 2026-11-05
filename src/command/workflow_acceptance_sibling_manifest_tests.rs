use super::*;

#[test]
fn explicit_sibling_manifests_resolve_inputs_with_equal_treatment() {
    let root = tempfile::tempdir().unwrap();
    let cwd = root.path().join("work");
    let sibling = root.path().join("library");
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::create_dir_all(sibling.join("tests")).unwrap();
    std::fs::write(sibling.join("tests/example.data"), "input").unwrap();
    for (manifest, command) in [
        ("Cargo.toml", "tool --manifest-path ../library/Cargo.toml"),
        ("pom.xml", "tool -f ../library/pom.xml"),
        ("package.json", "tool --config ../library/package.json"),
        ("pyproject.toml", "tool --config ../library/pyproject.toml"),
        ("go.mod", "tool --config ../library/go.mod"),
    ] {
        std::fs::write(sibling.join(manifest), "manifest").unwrap();
        assert_eq!(
            manifest_relative_input("tests/example.data", command, &cwd),
            Some("../library/tests/example.data".into())
        );
    }
    assert!(
        manifest_relative_input("tests/example.data", "echo ../library/pom.xml", &cwd).is_none()
    );
    std::fs::create_dir_all(cwd.join("tests")).unwrap();
    std::fs::write(cwd.join("tests/example.data"), "local").unwrap();
    assert!(
        manifest_relative_input("tests/example.data", "tool -f ../library/pom.xml", &cwd).is_none()
    );
}
