use super::*;

#[test]
fn raw_shell_statement_keeps_quoted_backticks_as_command_text() {
    let command = ": check ; printf 'error: unused import: `x0`'; exit 1";
    assert_eq!(
        focused_test_command_with_path(
            command,
            &[],
            std::path::Path::new(""),
            std::ffi::OsStr::new("")
        ),
        Some(command.into())
    );
}

pub(crate) fn command_site() -> (tempfile::TempDir, std::ffi::OsString) {
    let site = tempfile::tempdir().unwrap();
    for name in ["git", "cargo", "python3", "node", "bash", "test"] {
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

#[test]
fn spans_in_every_position_require_a_declared_or_resolved_program() {
    for span in [
        "`check --one`",
        "`check --one` prints a value",
        "run `check --one`",
        "run targeted `check --one`",
    ] {
        assert_eq!(
            focused_test_command_at(span, &["check".into()], std::path::Path::new("")).as_deref(),
            Some("check --one")
        );
    }
    for entry in [
        "`unregistered_case_name`",
        "verify parser handles migration",
        "`data list --json` prints the registered dataset",
    ] {
        assert!(
            focused_test_command_with_path(entry, &[], std::path::Path::new(""), &command_site().1)
                .is_none(),
            "{entry}"
        );
    }
}
#[cfg(unix)]
#[test]
fn repository_operands_require_executable_files_at_the_repository_root() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("scripts")).unwrap();
    let executable = root.path().join("scripts/check");
    std::fs::write(&executable, "#!/bin/sh\nexit 0\n").unwrap();
    assert!(focused_test_command_at("run `./scripts/check`", &[], root.path()).is_none());
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(
        focused_test_command_at("run `./scripts/check`", &[], root.path()).as_deref(),
        Some("./scripts/check")
    );
    assert!(
        std::process::Command::new(&executable)
            .status()
            .unwrap()
            .success()
    );
    assert!(focused_test_command_at("run `./scripts/missing`", &[], root.path()).is_none());
    assert!(focused_test_command_at("run `./scripts`", &[], root.path()).is_none());
}
#[test]
fn search_path_programs_are_commands_with_no_tool_declarations() {
    for entry in [
        "`git diff --check`",
        "run `git diff --check`",
        "run targeted `git diff --check`",
    ] {
        assert_eq!(
            focused_test_command_with_path(entry, &[], std::path::Path::new(""), &command_site().1)
                .as_deref(),
            Some("git diff --check")
        );
    }
}

#[test]
fn a_span_prefix_does_not_make_an_unavailable_program_runnable() {
    for entry in [
        "`MODE=x absent_site_program --one`",
        "run `! absent_site_program --one`",
        "`MODE=x`",
    ] {
        assert!(
            focused_test_command_with_path(entry, &[], std::path::Path::new(""), &command_site().1)
                .is_none(),
            "{entry}"
        );
    }
    for entry in [
        "`MODE=x git diff --check`",
        "`MODE='one two' git diff --check`",
        "`MODE=\"$(git)\"`",
        "run `! git diff --check`",
        "`MODE=x check --one`",
    ] {
        assert!(
            focused_test_command_with_path(
                entry,
                &["check".into()],
                std::path::Path::new(""),
                &command_site().1
            )
            .is_some(),
            "{entry}"
        );
    }
}

#[cfg(unix)]
#[test]
fn executable_paths_are_confined_to_the_repository() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    assert!(
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(root.path())
            .status()
            .unwrap()
            .success()
    );
    let outside = tempfile::tempdir().unwrap();
    for dir in [root.path(), outside.path()] {
        let path = dir.join("check");
        std::fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let inside = root.path().join("check");
    let external = outside.path().join("check");
    std::os::unix::fs::symlink(&external, root.path().join("escape")).unwrap();
    assert!(
        focused_test_command_at(&format!("run `{}`", inside.display()), &[], root.path()).is_some()
    );
    assert!(
        focused_test_command_at(&format!("run `{}`", external.display()), &[], root.path())
            .is_none()
    );
    assert!(focused_test_command_at("run `./escape`", &[], root.path()).is_none());
    assert!(
        std::process::Command::new(inside)
            .status()
            .unwrap()
            .success()
    );
}

#[cfg(unix)]
#[test]
fn repository_record_binds_scripts_beside_an_external_task_directory() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let tasks = root.path().join("task-set");
    let repository = root.path().join("source");
    std::fs::create_dir(&tasks).unwrap();
    std::fs::create_dir(&repository).unwrap();
    let script = repository.join("check");
    std::fs::write(&script, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(
        tasks.join("repository.lock"),
        serde_json::to_vec(&serde_json::json!({
            "schema_version": 1,
            "repository_root": repository,
            "base_commit": "unborn",
            "decomposition_run_id": "fixture",
            "recorded_at": "2026-11-08T00:00:00Z",
        }))
        .unwrap(),
    )
    .unwrap();
    let bound = repository_root(&tasks.join("TASK-X-001.md").to_string_lossy());
    assert_eq!(bound, repository);
    assert_eq!(
        focused_test_command_with_path("`./check`", &[], &bound, std::ffi::OsStr::new(""))
            .as_deref(),
        Some("./check")
    );
    assert!(
        focused_test_command_with_path(
            "`./check`",
            &[],
            std::path::Path::new(""),
            std::ffi::OsStr::new("")
        )
        .is_none()
    );
}
