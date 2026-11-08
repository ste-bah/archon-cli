use super::*;

#[test]
fn active_landlock_refuses_failed_operator_read_and_write_rules() {
    for reason in ["injected read failure", "injected write failure"] {
        let error = require_operator_boundary::<()>(Err(reason.into()), true)
            .expect_err("active Landlock must refuse a failed ruleset");
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert!(error.to_string().contains("It was NOT run"));
        assert_eq!(
            require_operator_boundary::<()>(Err(reason.into()), false).unwrap(),
            None
        );
    }
    assert_eq!(require_operator_boundary(Ok(7), true).unwrap(), Some(7));
}

#[test]
fn persistent_terminal_seals_relative_configuration_writes() {
    use std::path::PathBuf;
    const STAGE: &str = "ARCHON_PTY_SEAL_STAGE";
    const TEST: &str = "unix::tests::persistent_terminal_seals_relative_configuration_writes";
    if let Some(stage) = std::env::var_os(STAGE) {
        let root = PathBuf::from(std::env::var_os("HOME").unwrap());
        if stage == "sealed" {
            for path in [
                ".git/config",
                "../.bashrc",
                "../.gitconfig",
                "../.config/autostart/new",
                "../.archon/operator-env-grants/forged",
                "/proc/self/cwd/.git/config",
            ] {
                assert_eq!(
                    std::fs::write(path, "forged").unwrap_err().raw_os_error(),
                    Some(libc::EROFS),
                    "{path}"
                );
            }
            std::fs::write("ordinary", "allowed").unwrap();
            return;
        }
        if archon_shell::write_boundary::landlock::probe().is_err() {
            assert!(std::env::var_os("CI").is_none(), "CI requires Landlock");
            return;
        }
        std::fs::write(root.join(".bashrc"), "# original\n").unwrap();
        std::fs::write(root.join(".gitconfig"), "[user]\nname = Operator\n").unwrap();
        let repo = root.join("repo");
        std::fs::create_dir(&repo).unwrap();
        assert!(
            archon_shell::spawn::git_command(&repo)
                .args(["init", "-q"])
                .status()
                .unwrap()
                .success()
        );
        archon_shell::spawn::register_repository(&repo).unwrap();
        if let Err(error) = archon_shell::read_boundary::prepared_write_paths().and_then(|paths| {
            archon_shell::write_boundary::landlock::LandlockSandbox::build_sealed(&paths)
        }) {
            assert!(
                std::env::var_os("CI").is_none(),
                "CI requires mount seals: {error}"
            );
            eprintln!("PTY kernel test unavailable: {error}");
            return;
        }
        let pair = crate::native_pty_system()
            .openpty(crate::PtySize::default())
            .unwrap();
        let mut builder = crate::CommandBuilder::new(std::env::current_exe().unwrap());
        builder.args(["--exact", TEST, "--nocapture"]);
        builder.env(STAGE, "sealed");
        builder.cwd(&repo);
        builder.protect_operator_reads();
        let mut child = pair.slave.spawn_command(builder).unwrap();
        let status = child.wait().unwrap();
        assert!(status.success(), "{status:?}");
        assert!(repo.join("ordinary").exists());
        return;
    }
    let root = std::env::temp_dir().join(format!(
        "pty-seals-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&root).unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", TEST, "--nocapture"])
        .env(STAGE, "prepare")
        .env("HOME", &root)
        .env("USERPROFILE", &root)
        .env_remove("GIT_CONFIG_GLOBAL")
        .env_remove("GIT_CONFIG_SYSTEM")
        .env("XDG_CONFIG_HOME", root.join(".config"))
        .env("XDG_DATA_HOME", root.join(".local/share"))
        .output()
        .unwrap();
    std::fs::remove_dir_all(root).unwrap();
    assert!(output.status.success(), "{output:?}");
}

#[test]
fn optional_seal_failures_do_not_refuse_terminal_commands() {
    for active in [false, true] {
        let mut command = std::process::Command::new("/bin/true");
        let report = archon_shell::read_boundary::install_prepared_seals(
            &mut command,
            Err("unshare, errno 1".into()),
            active,
            &[],
        );
        assert!(report.contains("not-contained") && report.contains("unshare, errno 1"));
        assert!(command.status().unwrap().success());
    }
}

#[test]
fn spawn_command_continues_after_seal_preparation_failure() {
    const CASE: &str = "unix::tests::spawn_command_continues_after_seal_preparation_failure";
    const STAGE: &str = "PTY_PREPARATION_FAILURE_STAGE";
    if std::env::var_os(STAGE).is_none() {
        let root = std::env::temp_dir().join(format!("pty-fallback-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", CASE, "--nocapture"])
            .env(STAGE, "1")
            .env("HOME", &root)
            .env("USERPROFILE", &root)
            .env("XDG_CONFIG_HOME", root.join(".config"))
            .env("XDG_DATA_HOME", root.join(".local/share"))
            .env_remove("GIT_CONFIG_GLOBAL")
            .env_remove("GIT_CONFIG_SYSTEM")
            .output()
            .unwrap();
        std::fs::remove_dir_all(root).unwrap();
        assert!(output.status.success(), "{output:?}");
        return;
    }
    let home = std::path::PathBuf::from(std::env::var_os("HOME").unwrap());
    let target = home.join("configuration");
    std::fs::write(&target, "[user]\nname = Operator\n").unwrap();
    std::os::unix::fs::symlink(&target, home.join(".gitconfig")).unwrap();
    // This deterministically fails preparation before any namespace probe.
    let error = archon_shell::write_boundary::landlock::LandlockSandbox::build_sealed(&[
        home.join(".gitconfig")
    ])
    .unwrap_err();
    assert!(
        error.contains("directory entry could be replaced"),
        "{error}"
    );
    let pair = crate::native_pty_system()
        .openpty(crate::PtySize::default())
        .unwrap();
    let marker = home.join("executed");
    let mut builder = crate::CommandBuilder::new("/bin/sh");
    builder.args(["-c", "printf ran > \"$1\"", "probe"]);
    builder.arg(&marker);
    builder.protect_operator_reads();
    let mut child = pair.slave.spawn_command(builder).unwrap();
    assert!(child.wait().unwrap().success());
    assert_eq!(std::fs::read(marker).unwrap(), b"ran");
}
