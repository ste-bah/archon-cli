pub(super) fn command_site() -> (tempfile::TempDir, std::ffi::OsString) {
    let site = tempfile::tempdir().unwrap();
    for name in [
        "cargo",
        "python3",
        "python",
        "pytest",
        "go",
        "node",
        "npm",
        "mvn",
        "sh",
        "test",
        "check",
        "custom-check",
    ] {
        let file = site.path().join(name);
        std::fs::write(&file, "").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
    let path = std::env::join_paths([site.path()]).unwrap();
    (site, path)
}
