//! The PowerShell tool's child gets the jobserver policy after its host
//! environment overlay, as the Bash tool's does.
//!
//! No PowerShell is needed: a stand-in `pwsh` on PATH runs the command text
//! with `sh`, so the test observes the exact environment the tool built.
use super::*;

const CHILD: &str = "powershell::jobserver_tests::powershell_jobserver_child";

fn launch_with_jobserver(flags: [&str; 3]) {
    let mut pipe = [-1; 2];
    // SAFETY: pipe writes two descriptors into the array supplied here.
    assert_eq!(unsafe { libc::pipe(pipe.as_mut_ptr()) }, 0);
    use std::os::fd::FromRawFd;
    // SAFETY: both descriptors were just created and are owned only here.
    let _read = unsafe { std::fs::File::from_raw_fd(pipe[0]) };
    // SAFETY: as above.
    let _write = unsafe { std::fs::File::from_raw_fd(pipe[1]) };
    let pair = format!("{},{}", pipe[0], pipe[1]);
    let fake = tempfile::tempdir().unwrap();
    let pwsh = fake.path().join("pwsh");
    std::fs::write(
        &pwsh,
        "#!/bin/sh\n[ \"$1\" = -Command ] || exit 64\nexec /bin/sh -c \"$2\"\n",
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&pwsh, std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = format!(
        "{}:{}",
        fake.path().display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args(["--ignored", "--exact", CHILD, "--nocapture"])
        .env("PATH", path);
    for (name, value) in ["MAKEFLAGS", "MFLAGS", "GNUMAKEFLAGS"]
        .into_iter()
        .zip(flags)
    {
        command.env(name, value.replace("PAIR", &pair));
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn powershell_child_gets_no_inherited_jobserver_auth_pair() {
    launch_with_jobserver(["--jobserver-auth=PAIR -j2", "--silent", ""]);
}

#[test]
fn powershell_child_gets_no_inherited_legacy_jobserver_fds() {
    launch_with_jobserver(["--silent", "--jobserver-fds=PAIR -j2", ""]);
}

#[test]
fn powershell_child_keeps_fifo_jobservers_and_assignments() {
    launch_with_jobserver([
        "--silent --jobserver-auth PAIR",
        "",
        "--jobserver-auth=fifo:/tmp/archon-jobserver --jobserver-fds PAIR -- X=keep",
    ]);
}

#[tokio::test]
#[ignore = "host environment is supplied only to an isolated child"]
async fn powershell_jobserver_child() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("Makefile"),
        "all:\n\t@printf 'make-completed\\n'\n",
    )
    .unwrap();
    let ctx = ToolContext {
        working_dir: dir.path().to_path_buf(),
        ..Default::default()
    };
    let raw = "printf '%s\\n' \"$MAKEFLAGS\" \"$MFLAGS\" \"$GNUMAKEFLAGS\"; unset GNUMAKEFLAGS; make -f Makefile";
    let result = PowerShellTool::default()
        .execute(serde_json::json!({ "command": raw }), &ctx)
        .await;
    let seen = &result.content;
    assert!(!result.is_error, "{seen}");
    assert!(seen.contains("make-completed"), "{seen}");
    assert!(!seen.contains("jobserver unavailable"), "{seen}");
    assert!(!seen.contains("--jobserver-fds"), "{seen}");
    for line in seen
        .lines()
        .filter(|line| line.contains("--jobserver-auth"))
    {
        assert!(
            line.contains("--jobserver-auth=fifo:"),
            "stale fd auth: {seen}"
        );
    }
    if std::env::var("GNUMAKEFLAGS").unwrap().contains("fifo:") {
        assert!(seen.contains("--jobserver-auth=fifo:/tmp/archon-jobserver"));
        assert!(seen.contains("-- X=keep"), "{seen}");
    }
    assert!(seen.contains("--silent"), "{seen}");
}
