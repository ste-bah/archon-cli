#[test]
fn canonical_path_comparison_handles_equivalent_spellings() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("artifact.json");
    std::fs::create_dir(directory.path().join("nested")).unwrap();
    std::fs::write(&file, b"artifact").unwrap();
    let alternate_spelling = directory
        .path()
        .join("nested")
        .join("..")
        .join("artifact.json");

    assert_eq!(
        file.canonicalize().unwrap(),
        alternate_spelling.canonicalize().unwrap()
    );
}
