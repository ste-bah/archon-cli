use super::*;

fn child(name: &str, vars: &[(&str, &OsStr)]) -> std::process::Output {
    let mut command = crate::spawn::command(std::env::current_exe().unwrap());
    command.args([
        "--exact",
        &format!("spawn::host_config::contract_tests::{name}"),
        "--nocapture",
    ]);
    command.env("ARCHON_HOST_CONTRACT_CHILD", "1");
    command.envs(vars.iter().copied());
    command.output().unwrap()
}
fn is_child() -> bool {
    std::env::var_os("ARCHON_HOST_CONTRACT_CHILD").is_some()
}
fn passed(output: std::process::Output) {
    assert!(output.status.success(), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("1 passed"),
        "{output:?}"
    );
}

#[test]
fn empty_and_relative_path_entries_are_skipped_for_every_program() {
    if !is_child() {
        let original = std::env::var_os("PATH").unwrap();
        let mut entries = vec![PathBuf::new(), PathBuf::from("relative")];
        entries.extend(std::env::split_paths(&original));
        let path = std::env::join_paths(entries).unwrap();
        passed(child(
            "empty_and_relative_path_entries_are_skipped_for_every_program",
            &[("PATH", &path)],
        ));
        return;
    }
    initialize().unwrap();
    assert!(std::env::split_paths(startup_path().as_ref().unwrap()).all(|p| p.is_absolute()));
    for (name, path) in programs::recorded() {
        let mut command = crate::spawn::command(name);
        assert_eq!(Path::new(command.get_program()), path.as_path());
        crate::spawn::replace_environment(&mut command, [("PATH", "relative")]);
        assert_eq!(Path::new(command.get_program()), path.as_path());
    }
    let (_, report) = crate::read_boundary::command_with_report(OsStr::new("probe"));
    // Skipping is true regardless of whether this host can install a kernel boundary.
    assert!(
        report.contains("empty and relative entries are skipped"),
        "{report}"
    );
    for path in std::env::split_paths(startup_path().as_ref().unwrap()) {
        assert!(!protected_paths().contains(&path));
    }
}

#[test]
fn missing_program_is_an_absolute_missing_executable() {
    let command = crate::spawn::command("archon-nonexistent-host-contract-program");
    let path = Path::new(command.get_program());
    assert!(path.is_absolute());
    assert!(!path.exists());
    assert_eq!(
        path.parent(),
        Some(std::env::current_exe().unwrap().as_path())
    );
    assert!(std::fs::create_dir(path.parent().unwrap()).is_err());
    assert!(!path.to_string_lossy().contains("initialization-refused"));
}

#[test]
fn nonrepository_and_scratch_commands_never_register_the_tree() {
    if !is_child() {
        passed(child(
            "nonrepository_and_scratch_commands_never_register_the_tree",
            &[],
        ));
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let before = super::super::host_repository::protected_paths();
    let mut command = crate::spawn::git_command(root.path());
    assert!(
        !command
            .get_envs()
            .any(|(name, value)| name == "GIT_DIR" && value.is_some())
    );
    assert!(
        command
            .args(["init", "-q"])
            .output()
            .unwrap()
            .status
            .success()
    );
    let mut command = crate::spawn::git_command(root.path());
    assert!(
        !command
            .get_envs()
            .any(|(name, value)| name == "GIT_DIR" && value.is_some())
    );
    assert!(
        command
            .args(["config", "scratch.probe", "writable"])
            .output()
            .unwrap()
            .status
            .success()
    );
    crate::read_boundary::prepare_spawn(&command).unwrap();
    for _ in 0..8 {
        let scratch = tempfile::tempdir().unwrap();
        assert!(
            crate::spawn::git_command(scratch.path())
                .args(["init", "-q"])
                .output()
                .unwrap()
                .status
                .success()
        );
        assert!(
            crate::spawn::git_command(scratch.path())
                .args(["config", "scratch.probe", "writable"])
                .output()
                .unwrap()
                .status
                .success()
        );
    }
    let after = super::super::host_repository::protected_paths();
    assert!(!after.iter().any(|path| path.starts_with(root.path())));
    assert_eq!(
        after, before,
        "scratch repositories must not grow the registry or profile"
    );
}

#[test]
fn run_end_removes_snapshot_and_clears_registered_repositories() {
    if !is_child() {
        passed(child(
            "run_end_removes_snapshot_and_clears_registered_repositories",
            &[],
        ));
        return;
    }
    let run = crate::spawn::host_run();
    initialize().unwrap();
    let snapshot = host().unwrap().snapshot.clone();
    let directory = snapshot.parent().unwrap().to_path_buf();
    assert!(snapshot.is_file());
    let root = tempfile::tempdir().unwrap();
    assert!(
        crate::spawn::git_command(root.path())
            .args(["init", "-q"])
            .output()
            .unwrap()
            .status
            .success()
    );
    crate::spawn::register_repository(root.path()).unwrap();
    assert!(!super::super::host_repository::protected_paths().is_empty());
    drop(run);
    assert!(!directory.exists());
    assert!(super::super::host_repository::protected_paths().is_empty());
    let run = crate::spawn::host_run();
    initialize().unwrap();
    assert!(snapshot.is_file());
    drop(run);
    assert!(!directory.exists());
}

#[test]
fn registered_repository_seals_only_configuration_files() {
    let root = tempfile::tempdir().unwrap();
    assert!(
        crate::spawn::git_command(root.path())
            .args(["init", "-q"])
            .output()
            .unwrap()
            .status
            .success()
    );
    crate::spawn::register_repository(root.path()).unwrap();
    let paths = super::super::host_repository::protected_paths();
    let real = root.path().canonicalize().unwrap();
    assert!(paths.contains(&real.join(".git/config")));
    for path in [
        &real,
        &real.join(".git"),
        &real.join(".git/index.lock"),
        &real.join("target"),
        &real.join("new-file"),
    ] {
        assert!(
            !paths.iter().any(|sealed| path.starts_with(sealed)),
            "{path:?}: {paths:?}"
        );
    }
}

#[test]
fn missing_git_is_reported_without_refusing_an_available_program() {
    if !is_child() {
        let root = tempfile::tempdir().unwrap();
        passed(child(
            "missing_git_is_reported_without_refusing_an_available_program",
            &[("PATH", root.path().as_os_str())],
        ));
        return;
    }
    let error = initialize().unwrap_err();
    assert!(error.contains("Git is missing"), "{error}");
    let command = crate::spawn::command("git");
    let env: std::collections::BTreeMap<_, _> = command.get_envs().collect();
    assert_eq!(
        env[OsStr::new("GIT_CONFIG_GLOBAL")],
        Some(OsStr::new(neutral()))
    );
    let executable = std::env::current_exe().unwrap();
    let (mut command, report) = crate::read_boundary::command_with_report(executable.as_os_str());
    assert!(report.contains(&error), "{report}");
    assert!(!report.contains("launch-refused"), "{report}");
    assert!(command.arg("--list").output().unwrap().status.success());
}

#[test]
#[cfg(unix)]
fn deleted_cwd_is_reported_without_disabling_commands() {
    if !is_child() {
        passed(child(
            "deleted_cwd_is_reported_without_disabling_commands",
            &[],
        ));
        return;
    }
    let root = tempfile::tempdir().unwrap();
    std::env::set_current_dir(root.path()).unwrap();
    std::fs::remove_dir(root.path()).unwrap();
    let error = initialize().unwrap_err();
    assert!(error.contains("working directory unavailable"), "{error}");
    let (mut command, report) = crate::read_boundary::command_with_report(OsStr::new("git"));
    assert!(report.contains(&error), "{report}");
    assert!(!report.contains("launch-refused"), "{report}");
    assert!(command.arg("--version").status().unwrap().success());
    // Construction of unrelated host commands still succeeds.
    assert!(Path::new(crate::spawn::command("probe").get_program()).is_absolute());
}

#[test]
fn operator_origins_and_includes_are_files_discovered_by_git() {
    if !is_child() {
        let root = tempfile::tempdir().unwrap();
        let global = root.path().join("operator-global");
        let system = root.path().join("operator-system");
        let included = root.path().join("included");
        std::fs::write(
            &global,
            "[include]\npath = included\n[user]\nname = operator\n",
        )
        .unwrap();
        std::fs::write(&system, "[user]\nemail = system@example.test\n").unwrap();
        std::fs::write(&included, "[custom]\noriginal = flattened\n").unwrap();
        passed(child(
            "operator_origins_and_includes_are_files_discovered_by_git",
            &[
                ("GIT_CONFIG_GLOBAL", global.as_os_str()),
                ("GIT_CONFIG_SYSTEM", system.as_os_str()),
                ("ARCHON_HOST_ORIGINS", root.path().as_os_str()),
                ("GIT_CONFIG_NOSYSTEM", OsStr::new("0")),
            ],
        ));
        return;
    }
    initialize().unwrap();
    let root = PathBuf::from(std::env::var_os("ARCHON_HOST_ORIGINS").unwrap());
    let paths = protected_paths();
    for name in ["operator-global", "operator-system", "included"] {
        let path = root.join(name);
        assert!(paths.contains(&path), "{path:?}: {paths:?}");
        assert!(!crate::read_boundary::operator_paths().contains(&path));
        assert!(std::fs::read(&path).is_ok());
    }
    assert!(!paths.contains(&root));
    std::fs::write(root.join("included"), "[custom]\ninjected = planted\n").unwrap();
    let original = crate::spawn::command("git")
        .args(["config", "--global", "--get", "custom.original"])
        .output()
        .unwrap();
    assert!(original.status.success());
    assert_eq!(
        String::from_utf8_lossy(&original.stdout).trim(),
        "flattened"
    );
    let injected = crate::spawn::command("git")
        .args(["config", "--global", "--get", "custom.injected"])
        .output()
        .unwrap();
    assert_eq!(injected.status.code(), Some(1));
}

#[test]
fn registration_failure_is_reported_without_disabling_commands() {
    if !is_child() {
        passed(child(
            "registration_failure_is_reported_without_disabling_commands",
            &[],
        ));
        return;
    }
    let _run = crate::spawn::host_run();
    initialize().unwrap();
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join(".git"), "gitdir: absent\n").unwrap();
    let error = crate::spawn::register_repository(root.path()).unwrap_err();
    assert!(error.contains("repository capture failed"), "{error}");
    let (mut command, report) = crate::read_boundary::command_with_report(OsStr::new("git"));
    assert!(report.contains(&error), "{report}");
    assert!(!report.contains("launch-refused"), "{report}");
    assert!(command.arg("--version").status().unwrap().success());
    assert!(
        crate::spawn::command("git")
            .arg("--version")
            .output()
            .unwrap()
            .status
            .success()
    );
}

#[test]
fn changed_snapshot_uses_null_configuration() {
    if !is_child() {
        passed(child("changed_snapshot_uses_null_configuration", &[]));
        return;
    }
    initialize().unwrap();
    let snapshot = &host().unwrap().snapshot;
    std::fs::write(snapshot, "[custom]\ninjected = yes\n").unwrap();
    assert!(initialize().is_err());
    let mut command = crate::spawn::command("git");
    let env: std::collections::BTreeMap<_, _> = command.get_envs().collect();
    assert_eq!(
        env[OsStr::new("GIT_CONFIG_GLOBAL")],
        Some(OsStr::new(neutral()))
    );
    let output = command
        .args(["config", "--global", "--get", "custom.injected"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    std::fs::remove_file(snapshot).unwrap();
    std::fs::create_dir(snapshot).unwrap();
    let command = crate::spawn::command("git");
    let env: std::collections::BTreeMap<_, _> = command.get_envs().collect();
    assert_eq!(
        env[OsStr::new("GIT_CONFIG_GLOBAL")],
        Some(OsStr::new(neutral()))
    );
}

#[test]
fn absent_repository_include_flags_launch_even_after_planting() {
    if !is_child() {
        passed(child(
            "absent_repository_include_flags_launch_even_after_planting",
            &[],
        ));
        return;
    }
    let root = tempfile::tempdir().unwrap();
    assert!(
        crate::spawn::git_command(root.path())
            .args(["init", "-q"])
            .status()
            .unwrap()
            .success()
    );
    assert!(
        crate::spawn::git_command(root.path())
            .args(["config", "include.path", "../absent"])
            .status()
            .unwrap()
            .success()
    );
    crate::spawn::register_repository(root.path()).unwrap();
    let mut child_program = None;
    for planted in [false, true] {
        if planted {
            std::fs::write(root.path().join(".gitattributes"), "* filter=injected\n").unwrap();
            std::fs::write(root.path().join("payload"), "changed\n").unwrap();
            std::fs::write(
                root.path().join("absent"),
                "[filter \"injected\"]\nclean = \"echo executed > marker; cat\"\n",
            )
            .unwrap();
            for args in [["add", "-A"], ["diff", "--stat"]] {
                let mut command = crate::spawn::git_command(root.path());
                let result = command.args(args).status();
                assert!(
                    result.is_err(),
                    "refused command unexpectedly started: {result:?}"
                );
                assert!(!root.path().join("marker").exists());
                let result = crate::spawn::command("git")
                    .current_dir(root.path())
                    .args(args)
                    .status();
                assert!(
                    result.is_err(),
                    "refused command unexpectedly started: {result:?}"
                );
                assert!(!root.path().join("marker").exists());
            }
        }
        let (command, report) =
            crate::execution_environment::command_with_report(OsStr::new("git"));
        if let Some(program) = &child_program {
            assert_eq!(command.get_program(), program);
        } else {
            child_program = Some(command.get_program().to_owned());
        }
        assert!(
            report.contains("repository includes were absent at capture; host commands refuse observed child-created configuration:"),
            "{report}"
        );
        assert!(crate::read_boundary::observation_note(report).is_some());
    }
}
