use std::io::{Read, Seek, SeekFrom};

use clap::Parser;

#[cfg(unix)]
#[test]
fn write_budget_warning_is_emitted_once_during_real_bootstrap() {
    const NAME: &str =
        "main_bootstrap_tests::write_budget_warning_is_emitted_once_during_real_bootstrap";
    if crate::test_environment::isolated_named(NAME) {
        return;
    }

    let config_dir = tempfile::tempdir().expect("temporary config directory");
    let log_dir = config_dir.path().join("logs");
    std::fs::create_dir(&log_dir).expect("create temporary log directory");
    // SAFETY: this test is isolated in a child process before bootstrap starts threads.
    unsafe {
        std::env::set_var("ARCHON_LOG_DIR", log_dir);
    }
    let settings_path = config_dir.path().join("config.toml");
    std::fs::write(
        &settings_path,
        "[workflow.generated]\nhost_call_timeout_secs = 28800\nwrite_call_time_budget_secs = 28800\n",
    )
    .expect("write test configuration");

    let mut cli = crate::cli_args::Cli::try_parse_from([
        "archon",
        "--settings",
        settings_path.to_str().expect("UTF-8 test path"),
    ])
    .expect("parse bootstrap-only CLI arguments");
    cli.setting_sources = Some(Vec::new());
    let (stderr, _) = capture_stderr(|| {
        let _bootstrap = crate::main_bootstrap::bootstrap(&cli).expect("bootstrap succeeds");
    });
    let warning_count = stderr
        .lines()
        .filter(|line| {
            line.contains("warning: workflow.generated.write_call_time_budget_secs")
                && line.contains("workflow.generated.host_call_timeout_secs")
        })
        .count();
    assert_eq!(warning_count, 1, "stderr was:\n{stderr}");
}

#[cfg(unix)]
fn capture_stderr<T>(run: impl FnOnce() -> T) -> (String, T) {
    use std::os::fd::AsRawFd;

    let mut capture = tempfile::tempfile().expect("create stderr capture file");
    // SAFETY: stderr is a valid descriptor and capture owns a writable file descriptor.
    let saved_stderr = unsafe { libc::dup(libc::STDERR_FILENO) };
    assert!(saved_stderr >= 0);
    // SAFETY: redirect stderr to the temporary file.
    assert_eq!(
        unsafe { libc::dup2(capture.as_raw_fd(), libc::STDERR_FILENO) },
        libc::STDERR_FILENO
    );

    let result = run();

    // SAFETY: restore the original stderr and close the saved descriptor.
    assert_eq!(
        unsafe { libc::dup2(saved_stderr, libc::STDERR_FILENO) },
        libc::STDERR_FILENO
    );
    unsafe {
        libc::close(saved_stderr);
    }
    capture
        .seek(SeekFrom::Start(0))
        .expect("rewind captured stderr");
    let mut stderr = String::new();
    capture
        .read_to_string(&mut stderr)
        .expect("read captured stderr");
    (stderr, result)
}
