use super::*;

#[test]
fn registered_repository_include_and_authority_directory_are_kernel_sealed() {
    const STAGE: &str = "ARCHON_BOUNDARY_KERNEL_STAGE";
    const ROOT: &str = "ARCHON_BOUNDARY_KERNEL_ROOT";
    const TEST: &str = "read_boundary::kernel_tests::registered_repository_include_and_authority_directory_are_kernel_sealed";
    let stage = std::env::var(STAGE).unwrap_or_default();
    if stage.is_empty() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        std::fs::create_dir(root.join("home")).unwrap();
        std::fs::write(root.join("global"), "").unwrap();
        std::fs::write(root.join("system"), "").unwrap();
        let output = crate::spawn::command(std::env::current_exe().unwrap())
            .args(["--exact", TEST, "--nocapture"])
            .env(STAGE, "prepare")
            .env(ROOT, &root)
            .env("HOME", root.join("home"))
            .env("USERPROFILE", root.join("home"))
            .env("XDG_CONFIG_HOME", root.join("home/.config"))
            .env("XDG_DATA_HOME", root.join("home/.local/share"))
            .env("GIT_CONFIG_GLOBAL", root.join("global"))
            .env("GIT_CONFIG_SYSTEM", root.join("system"))
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        return;
    }
    let root = PathBuf::from(std::env::var_os(ROOT).unwrap());
    let repo = root.join("repo");
    let included = root.join("included");
    let startup = root.join("home/.config/autostart");
    if stage == "replace-link" {
        let link = root.join("home/.bashrc");
        std::fs::remove_file(&link).unwrap();
        std::fs::write(link, "forged").unwrap();
        return;
    }
    if stage == "restricted" {
        for path in [
            ".git/config",
            "../home/.profile",
            "../home/.archon/operator-env-grants/forged.json",
            "/proc/self/cwd/.git/config",
        ] {
            assert_eq!(
                std::fs::write(path, "forged").unwrap_err().raw_os_error(),
                Some(libc::EROFS),
                "{path}"
            );
        }
        for path in [
            repo.join(".git/config"),
            repo.join(".git/config.worktree"),
            included,
            root.join("absent-include"),
            startup.join("new"),
            root.join("home/.aws/credentials"),
            root.join("home/.config/git/config"),
            crate::credential_paths::operator_grant_root().join("forged.json"),
        ] {
            assert_eq!(
                std::fs::write(&path, "forged").unwrap_err().raw_os_error(),
                Some(libc::EROFS),
                "{}",
                path.display()
            );
        }
        assert_eq!(
            std::fs::rename(repo.join(".git"), repo.join("replaced"))
                .unwrap_err()
                .raw_os_error(),
            Some(libc::EBUSY)
        );
        std::fs::write(repo.join("ordinary"), "allowed").unwrap();
        for (source, target) in [
            (repo.join("ordinary"), root.join("home/moved")),
            (root.join("home/ordinary"), repo.join("moved")),
            (
                std::env::temp_dir().join(format!("boundary-{}", std::process::id())),
                repo.join("from-temp"),
            ),
        ] {
            std::fs::write(&source, "allowed").unwrap();
            assert_eq!(
                std::fs::rename(&source, &target)
                    .unwrap_err()
                    .raw_os_error(),
                Some(libc::EXDEV)
            );
            assert_eq!(
                std::fs::hard_link(&source, &target)
                    .unwrap_err()
                    .raw_os_error(),
                Some(libc::EXDEV)
            );
            std::fs::remove_file(source).unwrap();
        }
        std::fs::write(repo.join(".git/index.lock"), "allowed").unwrap();
        std::fs::write(root.join("executed"), "yes").unwrap();
        return;
    }
    if crate::write_boundary::landlock::probe().is_err() {
        assert!(std::env::var_os("CI").is_none(), "CI requires Landlock");
        return;
    }
    std::fs::create_dir(&repo).unwrap();
    assert!(
        crate::spawn::git_command(&repo)
            .args(["init", "-q"])
            .status()
            .unwrap()
            .success()
    );
    std::fs::write(&included, "[custom]\nvalue = original\n").unwrap();
    let absent_include = root.join("absent-include");
    for args in [
        vec!["config", "include.path", included.to_str().unwrap()],
        vec![
            "config",
            "--add",
            "include.path",
            absent_include.to_str().unwrap(),
        ],
        vec!["config", "extensions.worktreeConfig", "true"],
    ] {
        assert!(
            crate::spawn::git_command(&repo)
                .args(args)
                .status()
                .unwrap()
                .success()
        );
    }
    // Existing repository sources remain sealed; absent in-tree sources are never materialized.
    std::fs::write(repo.join(".git/config.worktree"), "").unwrap();
    std::fs::write(root.join("home/.gitconfig"), "").unwrap();
    std::fs::create_dir_all(root.join("home/.config/git")).unwrap();
    crate::spawn::register_repository(&repo).unwrap();
    assert!(!startup.exists());
    std::fs::write(root.join("home/.profile"), "original").unwrap();
    let (_, report) = command_with_report(std::env::current_exe().unwrap().as_os_str());
    assert!(!report.contains("launch-refused"), "{report}");
    if report.contains("writes: not-contained") {
        assert!(
            std::env::var_os("CI").is_none(),
            "CI requires mount seals: {report}"
        );
        eprintln!("kernel test unavailable: {report}");
        return;
    }
    assert_eq!(
        std::fs::read(repo.join(".git/config.worktree")).unwrap(),
        b""
    );
    assert_eq!(std::fs::read(&absent_include).unwrap(), b"");
    use std::os::unix::fs::PermissionsExt;
    for directory in [
        &startup,
        &root.join("home/.archon"),
        &crate::credential_paths::operator_grant_root(),
    ] {
        assert_eq!(
            std::fs::metadata(directory).unwrap().permissions().mode() & 0o777,
            0o700,
            "{}",
            directory.display()
        );
    }
    std::fs::create_dir_all(root.join("home/.aws")).unwrap();
    std::fs::write(root.join("home/.aws/credentials"), "original").unwrap();
    assert!(
        startup.is_dir(),
        "missing autostart directory must be prepared"
    );
    std::os::unix::fs::symlink("missing", startup.join("dangling")).unwrap();
    // Optional dangling dotfiles must not prevent preparation either.
    std::os::unix::fs::symlink("missing", root.join("home/.bashrc")).unwrap();
    crate::spawn::register_repository(&repo).unwrap();
    let (mut command, report) = command_with_report(std::env::current_exe().unwrap().as_os_str());
    if report.contains("launch-refused") || report.contains("all optional seals absent") {
        assert!(
            std::env::var_os("CI").is_none(),
            "CI requires mount seals: {report}"
        );
        eprintln!("kernel test unavailable: {report}");
        return;
    }
    assert!(
        report.contains("Absent startup files are not sealed"),
        "{report}"
    );
    let output = command
        .args(["--exact", TEST, "--nocapture"])
        .env(STAGE, "restricted")
        .current_dir(&repo)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(root.join("executed").exists());
    assert!(report.contains("all seal steps completed"), "{report}");
    assert!(
        report.contains("applied seals: [read-only") || report.contains("rename guard"),
        "{report}"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("included")).unwrap(),
        "[custom]\nvalue = original\n"
    );
    let link = root.join("home/.bashrc");
    std::fs::remove_file(&link).unwrap();
    std::os::unix::fs::symlink("../included", &link).unwrap();
    let (mut replace, fallback) = command_with_report(std::env::current_exe().unwrap().as_os_str());
    assert!(
        fallback.contains("writes: not-contained") && fallback.contains("symbolic link"),
        "{fallback}"
    );
    assert!(
        replace
            .args(["--exact", TEST])
            .env(STAGE, "replace-link")
            .output()
            .unwrap()
            .status
            .success()
    );
    assert_eq!(std::fs::read(&link).unwrap(), b"forged");
    std::fs::write(repo.join(".git/config"), "host remains writable").unwrap();
}
