//! Exercise the CLI startup boundary, before workflow launch persistence.
#![cfg(unix)]

fn refuses(case: &str, text: &str, source: &str) {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join(".archon");
    let user = root.path().join("user");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(&user).unwrap();
    let path = match source {
        "user" => user.join("config.toml"),
        "settings" => root.path().join("settings.toml"),
        _ => project.join("config.toml"),
    };
    std::fs::write(&path, text).unwrap();
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_archon"));
    cmd.current_dir(root.path())
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap())
        .env("HOME", root.path())
        .env("ARCHON_CONFIG_DIR", &user)
        .env("CARGO_BUILD_JOBS", "2")
        .env("RUST_TEST_THREADS", "4")
        .args([
            "--setting-sources",
            if source == "settings" { "user" } else { source },
        ]);
    if source == "settings" {
        cmd.arg("--settings").arg(&path);
    }
    let output = cmd
        .args(["workflow", "plan", "verify fixture"])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "{case}: startup used defaults: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !project.join("workflows").exists(),
        "{case}: launch marker was created"
    );
}

#[test]
fn r3_cli_malformed_project() {
    refuses(
        "project TOML",
        "[workflow.acceptance_execution]\nenvironment_allowlist=[\n",
        "project",
    );
}
#[test]
fn r3_cli_malformed_user() {
    refuses("user TOML", "[workflow.acceptance_execution\n", "user");
}
#[test]
fn r3_cli_malformed_settings() {
    refuses(
        "settings TOML",
        "[workflow.acceptance_execution]\nrepository=\n",
        "settings",
    );
}
#[test]
fn r3_cli_scalar_policy() {
    refuses(
        "scalar policy",
        "[workflow]\nacceptance_execution='invalid'\n",
        "project",
    );
}
#[test]
fn r3_cli_missing_policy_fields() {
    refuses(
        "missing fields",
        "[workflow.acceptance_execution]\nenvironment_allowlist=[]\n",
        "project",
    );
}
#[test]
fn r3_cli_scalar_workflow() {
    refuses("scalar workflow", "workflow='invalid'\n", "project");
}
