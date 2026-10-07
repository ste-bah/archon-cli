//! Exercise the CLI startup boundary, before workflow launch persistence.
#![cfg(unix)]

const LAUNCH_PRD: &str = "# Fixture\n\n## Requirements\n\n| ID | Requirement |\n|---|---|\n| REQ-FIX-001 | Implement a function. |\n\n## Acceptance Criteria\n\n| ID | Criterion |\n|---|---|\n| AC-FIX-001 | The function passes its focused test. |\n";

fn refuses(case: &str, text: &str, source: &str) {
    let root = tempfile::tempdir().unwrap();
    init_repository(root.path());
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
    std::fs::create_dir_all(root.path().join("prds")).unwrap();
    std::fs::create_dir_all(root.path().join("tasks")).unwrap();
    std::fs::write(root.path().join("prds/fixture.md"), LAUNCH_PRD).unwrap();
    let output = cmd
        .args([
            "workflow",
            "decompose",
            "--yes",
            "--repository",
            ".",
            "--prd",
            "prds/fixture.md",
            "--tasks",
            "tasks/fixture",
        ])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "{case}: startup used defaults: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let diagnostic = String::from_utf8_lossy(&output.stderr);
    assert!(
        (diagnostic.contains("configuration at") && diagnostic.contains("could not be parsed"))
            || (diagnostic.contains("config parse error") && diagnostic.contains("workflow")),
        "{case}: wrong rejection: {diagnostic}"
    );
    assert!(
        !diagnostic.contains("using defaults"),
        "{case}: {diagnostic}"
    );
    assert!(
        !project.join("workflows").exists(),
        "{case}: launch marker was created"
    );
    valid_policy_control_creates_run_marker();
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

fn valid_policy_control_creates_run_marker() {
    let root = tempfile::tempdir().unwrap();
    let scratch = tempfile::tempdir().unwrap();
    init_repository(root.path());
    for path in [".archon", "user", "prds", "tasks"] {
        std::fs::create_dir_all(root.path().join(path)).unwrap();
    }
    std::fs::write(root.path().join("prds/fixture.md"), LAUNCH_PRD).unwrap();
    let policy = format!(
        "[workflow]\ngate_mode='observe'\n[workflow.acceptance_execution]\nrepository={:?}\nscratch_parent={:?}\nproject_inputs=[]\nproject_repository_view='separate'\ntoolchain_path={:?}\nenvironment_allowlist=[]\ntimeout_secs=10\noutput_bytes=4096\nscratch_bytes=1048576\n",
        root.path().display().to_string(),
        scratch.path().display().to_string(),
        std::env::var("PATH").unwrap()
    );
    std::fs::write(root.path().join(".archon/config.toml"), policy).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_archon"))
        .current_dir(root.path())
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap())
        .env("HOME", root.path())
        .env("ARCHON_CONFIG_DIR", root.path().join("user"))
        .env("CARGO_BUILD_JOBS", "2")
        .env("RUST_TEST_THREADS", "4")
        .args([
            "--setting-sources",
            "project",
            "workflow",
            "decompose",
            "--yes",
            "--repository",
            ".",
            "--prd",
            "prds/fixture.md",
            "--tasks",
            "tasks/fixture",
        ])
        .output()
        .unwrap();
    let workflows = root.path().join(".archon/workflows");
    assert!(
        workflows.is_dir()
            && std::fs::read_dir(workflows).unwrap().any(|e| e
                .unwrap()
                .path()
                .join("state.json")
                .is_file()),
        "valid configuration never launched: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn init_repository(root: &std::path::Path) {
    let output = std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
