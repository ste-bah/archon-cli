//! Batch E: the run's project-input policy and where a landing may write.
use super::*;

fn project() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    let run_root = project.join(".archon/workflows/run1");
    std::fs::create_dir_all(&run_root).unwrap();
    std::fs::create_dir_all(project.join("tasks")).unwrap();
    (dir, project, run_root)
}

#[test]
fn the_policy_is_the_runs_own_and_never_a_copied_runs_project() {
    let (dir, project, run_root) = project();
    assert!(ProjectInputPolicy::for_run(&run_root).is_none());
    write_test_policy(&run_root, &project, &[".archon/lab"]);
    let policy = ProjectInputPolicy::for_run(&run_root).expect("recorded");
    assert_eq!(
        policy.project,
        project
            .canonicalize()
            .map(archon_shell::paths::plain)
            .unwrap()
    );
    assert_eq!(policy.inputs, [PathBuf::from(".archon/lab")]);

    // The same record in a run directory copied elsewhere names a project
    // that run is not inside: nothing is read from or written to it.
    let copy = dir.path().join("elsewhere/run1");
    std::fs::create_dir_all(copy.join("v2")).unwrap();
    std::fs::copy(
        run_root.join("v2/generated-metadata.json"),
        copy.join("v2/generated-metadata.json"),
    )
    .unwrap();
    assert!(ProjectInputPolicy::for_run(&copy).is_none());

    // No inputs: nothing to seed.
    write_test_policy(&run_root, &project, &[]);
    assert!(ProjectInputPolicy::for_run(&run_root).is_none());
}

#[test]
fn a_landing_writes_only_under_the_inputs_and_never_where_the_engine_loads() {
    let (_dir, project, run_root) = project();
    write_test_policy(&run_root, &project, &[".archon", "data", "tasks"]);
    let policy = ProjectInputPolicy::for_run(&run_root).unwrap();
    let root = project
        .canonicalize()
        .map(archon_shell::paths::plain)
        .unwrap();
    assert_eq!(
        policy.destination(".archon/lab/data/registry.json"),
        Ok(root.join(".archon/lab/data/registry.json"))
    );
    assert_eq!(
        policy.destination("data/x.bin"),
        Ok(root.join("data/x.bin"))
    );
    for refused in [
        ".archon/agents/coder.md",
        ".archon/Workflows/run1/state.json",
        ".archon/.hidden/x",
        ".archon/config.toml",
        ".archon/lab/data/.env",
        "data/.mcp.json",
        ".archon/lab",
        "data/../escape",
        "data/.git/config",
        "tasks/TASK-1.md",
        "src/lib.rs",
        "",
    ] {
        assert!(policy.destination(refused).is_err(), "{refused} accepted");
    }
    assert!(policy.covers("data/x.bin") && !policy.covers("database/x"));
}

#[test]
fn a_write_follows_no_link_and_reports_what_it_replaced() {
    let (dir, project, _) = project();
    let outside = dir.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::create_dir_all(project.join("data")).unwrap();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&outside, project.join("data/link")).unwrap();
        assert!(write_file(&project, &project.join("data/link/x"), b"x").is_err());
        assert!(!outside.join("x").exists());
    }
    let target = project.join("data/new/file.json");
    assert_eq!(write_file(&project, &target, b"one").unwrap(), None);
    assert_eq!(
        write_file(&project, &target, b"two").unwrap(),
        Some(b"one".to_vec())
    );
    assert_eq!(
        file_state(&target),
        blake3::hash(b"two").to_hex().to_string()
    );
    assert_eq!(file_state(&project.join("data/absent")), "absent");
}
