//! The final Bash environment, after real preparation has copied the host.
use super::*;

const CHILD: &str = "bash::bash_process_tests::jobserver_tests::prepared_jobserver_child";

fn launch_with_jobserver(flags: [&str; 3]) {
    let mut pipe = [-1; 2];
    assert_eq!(unsafe { libc::pipe(pipe.as_mut_ptr()) }, 0);
    // Real inherited jobserver pipe, kept alive until the isolated test exits.
    use std::os::fd::FromRawFd;
    let _read = unsafe { std::fs::File::from_raw_fd(pipe[0]) };
    let _write = unsafe { std::fs::File::from_raw_fd(pipe[1]) };
    let pair = format!("{},{}", pipe[0], pipe[1]);
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command.args(["--ignored", "--exact", CHILD, "--nocapture"]);
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
fn bash_preparation_removes_jobserver_auth_after_host_overlay() {
    launch_with_jobserver(["--jobserver-auth=PAIR -j2", "--silent", ""]);
}
#[test]
fn bash_preparation_removes_legacy_jobserver_fds_after_host_overlay() {
    launch_with_jobserver(["--silent", "--jobserver-fds=PAIR -j2", ""]);
}
#[test]
fn bash_preparation_removes_split_fds_and_keeps_fifo_and_assignments() {
    launch_with_jobserver([
        "--silent --jobserver-auth PAIR",
        "",
        "--jobserver-auth=fifo:/tmp/archon-jobserver --jobserver-fds PAIR -- X=keep",
    ]);
}

#[tokio::test]
#[ignore = "host environment is supplied only to an isolated child"]
async fn prepared_jobserver_child() {
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
    let tool = BashTool::default();
    let raw = "printf '%s\\n' \"$MAKEFLAGS\" \"$MFLAGS\" \"$GNUMAKEFLAGS\"; unset GNUMAKEFLAGS; make -f Makefile";
    let prepared = prepare_command(&tool, raw, 10_000, &ctx).await.unwrap();
    // Launch exactly the production command, including env_clear/envs.
    let result = run_prepared_bash_command(&tool, &ctx, raw, prepared).await;
    assert!(!result.is_error, "{}", result.content);
    assert!(
        result.content.contains("make-completed"),
        "{}",
        result.content
    );
    assert!(
        !result.content.contains("jobserver unavailable"),
        "{}",
        result.content
    );
    assert!(
        !result.content.contains("--jobserver-fds"),
        "{}",
        result.content
    );
    let expected_fifo = std::env::var("GNUMAKEFLAGS").unwrap().contains("fifo:");
    for line in result
        .content
        .lines()
        .filter(|line| line.contains("--jobserver-auth"))
    {
        assert!(
            line.contains("--jobserver-auth=fifo:"),
            "stale fd auth: {}",
            result.content
        );
    }
    if expected_fifo {
        assert!(
            result
                .content
                .contains("--jobserver-auth=fifo:/tmp/archon-jobserver")
        );
        assert!(result.content.contains("-- X=keep"));
    }
    assert!(result.content.contains("--silent"));
}
