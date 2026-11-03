//! Real launch regressions. Parent exports are installed in a separate test
//! process so concurrent tests never mutate the harness environment.
use crate::tool::{Tool, ToolContext};
#[cfg(unix)]
use std::time::Duration;

fn child(name: &str) -> bool {
    if std::env::var("ARCHON_TEST_SECRET").as_deref() == Ok("sk-test-355") {
        return true;
    }
    let status = archon_shell::spawn::command(std::env::current_exe().unwrap())
        .args([
            "--exact",
            &format!("agent_environment_tests::{name}"),
            "--nocapture",
        ])
        .env("ARCHON_TEST_SECRET", "sk-test-355")
        .env("LANG", "C")
        .env("TMPDIR", std::env::temp_dir())
        .status()
        .unwrap();
    assert!(status.success(), "agent environment regression: {name}");
    false
}

#[cfg(unix)]
const PROBE: &str = "if [ -z \"${ARCHON_TEST_SECRET+x}\" ]; then echo withheld; fi; if [ -n \"$PATH\" ] && [ -n \"$HOME\" ] && [ \"$LANG\" = C ] && [ -n \"$TMPDIR\" ]; then echo safe; fi";

fn check(output: &str, safe: bool) {
    assert!(
        output.contains(if safe { "safe" } else { "withheld" }),
        "required environment property absent"
    );
}

#[cfg(unix)]
async fn tool_probe(class: &str, safe: bool) {
    let ctx = ToolContext {
        working_dir: std::env::current_dir().unwrap(),
        ..ToolContext::default()
    };
    let tool: Box<dyn Tool> = match class {
        "bash" => Box::new(crate::bash::BashTool::default()),
        "monitor" => Box::new(crate::monitor::MonitorTool),
        _ => unreachable!(),
    };
    let result = tool
        .execute(serde_json::json!({"command": PROBE}), &ctx)
        .await;
    check(&result.content, safe);
}

macro_rules! tool_test {
    ($name:ident, $class:literal, $safe:literal) => {
        #[tokio::test]
        #[cfg(unix)]
        async fn $name() {
            if child(stringify!($name)) {
                tool_probe($class, $safe).await;
            }
        }
    };
}
tool_test!(agent_environment_bash_secret, "bash", false);
tool_test!(agent_environment_bash_safe, "bash", true);
tool_test!(agent_environment_monitor_secret, "monitor", false);
tool_test!(agent_environment_monitor_safe, "monitor", true);

#[cfg(unix)]
async fn java_probe(safe: bool) {
    use crate::java::project::{BuildSystem, JavaProject, Launcher, Stage};
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let wrapper = dir.path().join("gradlew");
    std::fs::write(&wrapper, format!("#!/bin/sh\n{PROBE}\n")).unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o700)).unwrap();
    let project = JavaProject {
        root: dir.path().into(),
        build_system: BuildSystem::Gradle,
        launcher: Launcher::Wrapper(wrapper),
    };
    let result =
        crate::java::invoke::run_stage(&project, Stage::Compile, Duration::from_secs(30), None)
            .await;
    assert!(result.succeeded());
    check(&result.output, safe);
}

#[cfg(unix)]
async fn pty_probe(safe: bool) {
    let mut command = crate::terminal_shell::build("bash", std::path::Path::new(".")).unwrap();
    command.args(["-c", PROBE]);
    let session = archon_pty::PtySession::spawn_headless(
        command,
        archon_pty::PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        },
    )
    .unwrap();
    let (_control, mut output) = session.split();
    let mut text = Vec::new();
    while let Some(bytes) = output.recv().await {
        text.extend(bytes);
    }
    check(&String::from_utf8_lossy(&text), safe);
}

macro_rules! launch_test {
    ($name:ident, $probe:ident, $safe:literal) => {
        #[tokio::test]
        #[cfg(unix)]
        async fn $name() {
            if child(stringify!($name)) {
                $probe($safe).await;
            }
        }
    };
}
launch_test!(agent_environment_java_secret, java_probe, false);
launch_test!(agent_environment_java_safe, java_probe, true);
launch_test!(agent_environment_pty_secret, pty_probe, false);
launch_test!(agent_environment_pty_safe, pty_probe, true);

#[cfg(windows)]
async fn powershell_probe(safe: bool) {
    let result = crate::powershell::PowerShellTool::default().execute(
        serde_json::json!({"command": "if (-not (Test-Path Env:ARCHON_TEST_SECRET)) { 'withheld' }; if ($env:PATH -and $env:USERPROFILE) { 'safe' }"}),
        &ToolContext { working_dir: std::env::current_dir().unwrap(), ..ToolContext::default() },
    ).await;
    check(&result.content, safe);
}

#[tokio::test]
#[cfg(windows)]
async fn agent_environment_powershell_secret() {
    if child("agent_environment_powershell_secret") {
        powershell_probe(false).await;
    }
}
#[tokio::test]
#[cfg(windows)]
async fn agent_environment_powershell_safe() {
    if child("agent_environment_powershell_safe") {
        powershell_probe(true).await;
    }
}

#[cfg(unix)]
#[tokio::test]
async fn agent_environment_pty_read_authority() {
    if archon_shell::write_boundary::mechanism().is_err() {
        eprintln!("kernel read boundary unavailable on this test host");
        return;
    }
    if let Some(path) = std::env::var_os("ARCHON_TEST_CREDENTIAL_PATH") {
        let mut command = crate::terminal_shell::build("bash", std::path::Path::new(".")).unwrap();
        command.args([
            "-c",
            "if cat \"$1\" >/dev/null 2>&1; then echo leaked; else echo denied; fi",
            "probe",
        ]);
        command.arg(path);
        let session = archon_pty::PtySession::spawn_headless(
            command,
            archon_pty::PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            },
        )
        .unwrap();
        let (_control, mut output) = session.split();
        let mut text = Vec::new();
        while let Some(bytes) = output.recv().await {
            text.extend(bytes);
        }
        assert!(
            String::from_utf8_lossy(&text).contains("denied"),
            "credential read must be denied"
        );
        return;
    }
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir(home.path().join(".aws")).unwrap();
    let path = home.path().join(".aws/credentials");
    std::fs::write(&path, "sk-test-file-355").unwrap();
    let status = archon_shell::spawn::command(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "agent_environment_tests::agent_environment_pty_read_authority",
            "--nocapture",
        ])
        .env("HOME", home.path())
        .env("ARCHON_TEST_CREDENTIAL_PATH", path)
        .status()
        .unwrap();
    assert!(status.success(), "PTY read authority regression");
}
