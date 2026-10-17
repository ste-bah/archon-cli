use std::process::Command;

#[test]
fn init_only_installs_profiles_and_exits_without_a_terminal() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let project = temp.path().join("project");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&project).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_archon"))
        .arg("--init-only")
        .current_dir(&project)
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_DATA_HOME", home.join("data"))
        .env("XDG_CACHE_HOME", home.join("cache"))
        .output()
        .expect("run archon --init-only");

    assert!(
        output.status.success(),
        "--init-only should exit successfully without a TTY; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        project.join(".archon/agents").is_dir(),
        "--init-only should install the bundled pipeline profiles"
    );
}
