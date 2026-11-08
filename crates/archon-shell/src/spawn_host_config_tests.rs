use super::*;
#[test]
fn rejects_git_versions_without_config_environment_support() {
    assert!(!supported_version("git version 2.31.9"));
    assert!(supported_version("git version 2.32.0"));
    assert!(supported_version("git version 2.50.1 (Apple Git-155)"));
}
#[test]
fn host_configuration_survives_environment_replacement() {
    initialize().unwrap();
    let mut command = crate::spawn::command("git");
    crate::spawn::replace_environment(
        &mut command,
        [("GIT_CONFIG_GLOBAL", "planted"), ("GIT_CONFIG_COUNT", "0")],
    );
    let env: std::collections::BTreeMap<_, _> = command.get_envs().collect();
    assert_ne!(
        env[OsStr::new("GIT_CONFIG_GLOBAL")],
        Some(OsStr::new("planted"))
    );
    assert_eq!(
        env[OsStr::new("GIT_CONFIG_NOSYSTEM")],
        Some(OsStr::new("1"))
    );
    assert_ne!(env[OsStr::new("GIT_CONFIG_COUNT")], Some(OsStr::new("0")));
    assert!(Path::new(command.get_program()).is_absolute());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&host().unwrap().snapshot)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}
#[test]
fn host_git_ignores_repository_monitor_and_rewritten_worktree_pointer() {
    let root = tempfile::tempdir().unwrap();
    let tree = root.path().join("tree");
    let other = root.path().join("other");
    for path in [&tree, &other] {
        assert!(
            crate::spawn::command("git")
                .arg("init")
                .arg(path)
                .output()
                .unwrap()
                .status
                .success()
        );
    }
    crate::spawn::register_repository(&tree).unwrap();
    std::fs::rename(tree.join(".git"), tree.join("saved-git")).unwrap();
    std::fs::write(
        tree.join(".git"),
        format!("gitdir: {}\n", other.join(".git").display()),
    )
    .unwrap();
    // A pinned selection fails when its actual directory is removed, rather than following the forged pointer.
    let output = crate::spawn::git_command(&tree)
        .arg("status")
        .output()
        .unwrap();
    assert!(!output.status.success());
    std::fs::remove_file(tree.join(".git")).unwrap();
    std::fs::rename(tree.join("saved-git"), tree.join(".git")).unwrap();
    let marker = root.path().join("executed");
    crate::spawn::command("git")
        .current_dir(&tree)
        .args(["config", "core.fsmonitor"])
        .arg(format!("echo planted > '{}'", marker.display()))
        .output()
        .unwrap();
    assert!(
        crate::spawn::git_command(&tree)
            .arg("status")
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(!marker.exists());
}

#[test]
#[cfg(unix)]
fn operator_global_config_is_a_snapshot_including_filters() {
    const PROBE: &str = "ARCHON_HOST_GIT_SNAPSHOT_TEST";
    if std::env::var_os(PROBE).is_none() {
        let root = tempfile::tempdir().unwrap();
        let include = root.path().join("included.config");
        std::fs::write(
            &include,
            "[filter \"operator\"]\nclean = cat\nsmudge = cat\n",
        )
        .unwrap();
        let global = root.path().join("global.config");
        std::fs::write(
            &global,
            format!(
                "[core]\nautocrlf = true\n[include]\npath = {}\n",
                include.display()
            ),
        )
        .unwrap();
        let output = crate::spawn::command(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "spawn::host_config::tests::operator_global_config_is_a_snapshot_including_filters",
                "--nocapture",
            ])
            .env(PROBE, &global)
            .env("GIT_CONFIG_GLOBAL", &global)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
        return;
    }
    initialize().unwrap();
    let global = PathBuf::from(std::env::var_os(PROBE).unwrap());
    std::fs::write(&global, "[core]\nautocrlf = false\n").unwrap();
    std::fs::write(
        global.parent().unwrap().join("included.config"),
        "[filter \"operator\"]\nclean = planted\n",
    )
    .unwrap();
    for (key, expected) in [("core.autocrlf", "true"), ("filter.operator.clean", "cat")] {
        let output = crate::spawn::command("git")
            .args(["config", "--global", "--get", key])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), expected);
    }
}

#[test]
fn host_path_lookup_is_not_repeated_after_environment_overlay() {
    initialize().unwrap();
    let planted = tempfile::tempdir().unwrap();
    std::fs::write(planted.path().join("git"), "planted").unwrap();
    let mut command = crate::spawn::command("git");
    crate::spawn::replace_environment(&mut command, [("PATH", planted.path().as_os_str())]);
    assert_eq!(Path::new(command.get_program()), host().unwrap().git);
    assert!(command.arg("--version").output().unwrap().status.success());
    if let Some(path) = startup_path() {
        for directory in std::env::split_paths(path) {
            assert!(!protected_paths().contains(&directory));
        }
    }
}

#[test]
fn repository_include_files_are_write_sealed_before_checks() {
    let root = tempfile::tempdir().unwrap();
    assert!(
        crate::spawn::command("git")
            .arg("init")
            .arg(root.path())
            .output()
            .unwrap()
            .status
            .success()
    );
    let include = root.path().join("tool-config");
    std::fs::write(&include, "[core]\nfsmonitor = false\n").unwrap();
    assert!(
        crate::spawn::command("git")
            .current_dir(root.path())
            .args(["config", "include.path"])
            .arg("../tool-config")
            .output()
            .unwrap()
            .status
            .success()
    );
    let nested = root.path().join("nested");
    std::fs::create_dir(&nested).unwrap();
    crate::spawn::register_repository(&nested).unwrap();
    let paths = crate::spawn::host_protected_paths();
    assert!(
        paths
            .iter()
            .any(|path| path.canonicalize().ok() == include.canonicalize().ok())
    );
}

#[test]
#[cfg(unix)]
fn unsupported_git_is_reported_without_refusing_an_available_program() {
    const PROBE: &str = "ARCHON_OLD_GIT_TEST";
    if std::env::var_os(PROBE).is_none() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let git = root.path().join("git");
        std::fs::write(&git, "#!/bin/sh\nprintf 'git version 2.31.9\\n'\n").unwrap();
        std::fs::set_permissions(&git, std::fs::Permissions::from_mode(0o700)).unwrap();
        let output = crate::spawn::command(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "spawn::host_config::tests::unsupported_git_is_reported_without_refusing_an_available_program",
                "--nocapture",
            ])
            .env(PROBE, "1")
            .env("PATH", root.path())
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
        return;
    }
    let error = initialize().unwrap_err();
    assert!(error.contains("requires version 2.32 or newer"), "{error}");
    assert!(error.contains("Git <2.32 ignores GIT_CONFIG_GLOBAL; operator global configuration is read; execution keys pinned by -c"));
    let pinned = crate::spawn::command("git");
    let args: Vec<_> = pinned
        .get_args()
        .map(|s| s.to_string_lossy().into_owned())
        .collect();
    for (key, value) in pins() {
        assert!(
            args.windows(2)
                .any(|pair| pair == ["-c", &format!("{key}={value}")])
        );
    }
    assert_ne!(
        pinned
            .get_envs()
            .find(|(key, _)| *key == "GIT_CONFIG_COUNT")
            .unwrap()
            .1,
        Some(OsStr::new("0"))
    );

    let executable = std::env::current_exe().unwrap();
    let (mut command, report) = crate::read_boundary::command_with_report(executable.as_os_str());
    assert!(report.contains(&error), "{report}");
    assert!(!report.contains("launch-refused"), "{report}");
    assert!(command.arg("--list").output().unwrap().status.success());
}

#[test]
fn existing_linked_checkout_can_be_registered_without_worktree_config_extension() {
    // This checkout may be a linked worktree. Git rejects --worktree config
    // inspection when multiple trees exist and the extension is disabled.
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    crate::spawn::register_repository(root).unwrap();
    assert!(
        crate::spawn::git_command(root)
            .args(["rev-parse", "--git-dir"])
            .output()
            .unwrap()
            .status
            .success()
    );
}
