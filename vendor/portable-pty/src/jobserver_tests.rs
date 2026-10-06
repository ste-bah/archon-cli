use crate::{native_pty_system, CommandBuilder, PtySize};

fn inherited(name: &str, value: &str, expected: &str) {
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "jobserver_tests::terminal_child", "--ignored"])
        .env("ARCHON_JOBSERVER_VARIABLE", name)
        .env("ARCHON_JOBSERVER_EXPECTED", expected)
        .env(name, value)
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("running 1 test"),
        "child filter did not run exactly one fixture"
    );
    assert!(
        output.status.success(),
        "{name}: {} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn terminal_strips_inherited_makeflags_auth_but_keeps_assignments() {
    inherited(
        "MAKEFLAGS",
        "-k -j2 --jobserver-auth=3,4 NAME=hello\\ world",
        "-k -j2 NAME=hello\\ world",
    );
    inherited(
        "MAKEFLAGS",
        "-k --jobserver-auth=3,4 -- --jobserver-auth=5,6",
        "-k -- --jobserver-auth=5,6",
    );
}
#[test]
fn terminal_strips_inherited_mflags_legacy_fds() {
    inherited("MFLAGS", "-s --jobserver-fds=5,6 -l3", "-s -l3");
}
#[test]
fn terminal_strips_inherited_gnumakeflags_separated_and_duplicate_fds() {
    inherited(
        "GNUMAKEFLAGS",
        "--jobserver-auth 7,8 -k --jobserver-fds=9,10 --jobserver-auth=fifo:/tmp/jobs",
        "-k --jobserver-auth=fifo:/tmp/jobs",
    );
}

#[test]
#[ignore = "isolated ambient terminal environment"]
fn terminal_child() {
    let Ok(name) = std::env::var("ARCHON_JOBSERVER_VARIABLE") else {
        return;
    };
    let expected = std::env::var("ARCHON_JOBSERVER_EXPECTED").unwrap();
    let pair = native_pty_system().openpty(PtySize::default()).unwrap();
    // Build from the inherited environment, exactly as web terminal shells do.
    let mut command = CommandBuilder::new("/bin/sh");
    assert_eq!(
        command.get_env(&name),
        Some(std::ffi::OsStr::new(&expected)),
        "the copied base environment still advertises closed descriptors"
    );
    // Explicit overrides must follow the same policy as inherited values.
    command.env(&name, std::env::var_os(&name).unwrap());
    assert_eq!(
        command.get_env(&name),
        Some(std::ffi::OsStr::new(&expected))
    );
    command.args(["-c", &format!("printf '%s' \"${name}\"")]);
    let mut child = pair.slave.spawn_command(command).unwrap();
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().unwrap();
    let mut output = Vec::new();
    let mut buffer = [0; 256];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => {
                output.extend_from_slice(&buffer[..n]);
                assert!(output.len() <= 2048);
            }
            Err(error) if error.raw_os_error() == Some(libc::EIO) => break,
            Err(error) => panic!("read terminal output: {}", error),
        }
    }
    assert!(child.wait().unwrap().success());
    assert_eq!(String::from_utf8(output).unwrap(), expected);
}
