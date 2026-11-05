use super::*;

#[test]
fn worktree_git_files_and_project_manifests_discover_the_root() {
    for marker in [
        ".git",
        "package.json",
        "pyproject.toml",
        "go.mod",
        "pom.xml",
        "Cargo.toml",
    ] {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("src/nested")).unwrap();
        std::fs::write(root.path().join(marker), "marker").unwrap();
        assert_eq!(
            repository_root_for_path(&root.path().join("src/nested")),
            Some(root.path().to_path_buf())
        );
    }
}
