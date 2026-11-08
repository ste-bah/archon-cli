use super::*;

#[test]
fn no_host_stamp_means_no_boundary() {
    let layout = layout();
    let worktree = layout.worktree.to_str().unwrap();
    let unstamped = WorkflowReadGuard::new(40, 20, false, false).with_declared_targets(
        DeclaredTargetScope::new(&["src/lib.rs".to_string()], Some(worktree))
            .in_isolated_worktree(true),
    );
    assert_eq!(unstamped.boundary_paths(), None);
    assert_eq!(
        unstamped.before_tool("Write", &json!({"file_path": path_str(&layout.data)})),
        None
    );
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn existing_user_configuration_remains_sealed_in_a_bounded_call() {
    const CASE: &str = "existing_user_configuration_remains_sealed_in_a_bounded_call";
    if std::env::var_os("ARCHON_FILE_ROOT_CASE").is_none() {
        let home = tempfile::tempdir().unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([CASE, "--nocapture"])
            .env("ARCHON_FILE_ROOT_CASE", "1")
            .env("HOME", home.path())
            .env("USERPROFILE", home.path())
            .env("XDG_CONFIG_HOME", home.path().join(".config"))
            .env("XDG_DATA_HOME", home.path().join(".local/share"))
            .env_remove("GIT_CONFIG_GLOBAL")
            .env_remove("GIT_CONFIG_SYSTEM")
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        return;
    }
    if !landlock() {
        assert!(std::env::var_os("CI").is_none(), "CI requires Landlock");
        return;
    }
    let home = PathBuf::from(std::env::var_os("HOME").unwrap());
    let file = home.join(".archon/config.toml");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, "original").unwrap();
    let layout = layout();
    let ctx = branch_ctx(&layout, true);
    let allowed = bash(&ctx, "printf allowed > src/probe").await;
    assert!(!allowed.is_error, "{}", allowed.content);
    let denied = bash(&ctx, &format!("printf forged > '{}'", file.display())).await;
    assert!(denied.is_error, "{}", denied.content);
    assert_eq!(read(&file), "original");
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn bash_reports_applied_seals_after_spawn() {
    const CASE: &str = "bash_reports_applied_seals_after_spawn";
    if std::env::var_os("BASH_SEAL_REPORT_CASE").is_none() {
        let home = tempfile::tempdir().unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([CASE, "--nocapture"])
            .env("BASH_SEAL_REPORT_CASE", "1")
            .env("HOME", home.path())
            .env("USERPROFILE", home.path())
            .env("XDG_CONFIG_HOME", home.path().join(".config"))
            .env("XDG_DATA_HOME", home.path().join(".local/share"))
            .env_remove("GIT_CONFIG_GLOBAL")
            .env_remove("GIT_CONFIG_SYSTEM")
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        return;
    }
    if let Err(reason) = archon_shell::read_boundary::prepared_write_paths() {
        assert!(
            std::env::var_os("CI").is_none(),
            "CI requires seals: {reason}"
        );
        eprintln!("kernel test unavailable: {reason}");
        return;
    }
    let (mut command, report) =
        super::super::bash_containment::contained_bash_command("printf executed", None);
    let output = command.output().await.unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"executed");
    assert!(report.contains("applied seals:"), "{report}");
    assert!(!report.contains("pending"), "{report}");
}
