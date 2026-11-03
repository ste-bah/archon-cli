use super::shell_quote;

pub(super) fn command(mode: &str) -> String {
    format!(
        "PROCESS_PROBE_MODE={mode} {} --exact {}::probe --nocapture",
        shell_quote(&std::env::current_exe().unwrap()),
        module_path!().split_once("::").unwrap().1,
    )
}

// Run this test binary inside the tool's boundary, without a setuid utility.
#[test]
fn probe() {
    let Ok(mode) = std::env::var("PROCESS_PROBE_MODE") else {
        return;
    };
    match mode.as_str() {
        "group" => {
            // SAFETY: getpgrp has no arguments or memory access.
            println!("\n{}", unsafe { libc::getpgrp() });
        }
        "session" => {
            // SAFETY: getsid(0) queries this process's session.
            let session = unsafe { libc::getsid(0) };
            assert!(
                session > 0,
                "session probe failed: {}",
                std::io::Error::last_os_error()
            );
            println!("\n{session}");
        }
        "stopped" => stopped_child(),
        _ => panic!("unknown process probe mode: {mode}"),
    }
}

#[expect(
    clippy::zombie_processes,
    reason = "the stopped child must outlive this parent so tool cleanup can prove descendant reaping"
)]
fn stopped_child() {
    let delayed = std::env::var_os("PROCESS_PROBE_DELAYED_FILE").unwrap();
    let pid_file = std::env::var_os("PROCESS_PROBE_PID_FILE").unwrap();
    let mut child = archon_shell::spawn::command("/bin/sh")
        .args(["-c", "kill -STOP \"$$\"; printf delayed > \"$1\"", "probe"])
        .arg(delayed)
        .spawn()
        .expect("could not start stopped-child probe");
    std::fs::write(pid_file, child.id().to_string()).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let mut status = 0;
        // SAFETY: status is writable and the pid names our own child.
        let result = unsafe {
            libc::waitpid(
                child.id() as i32,
                &mut status,
                libc::WUNTRACED | libc::WNOHANG,
            )
        };
        if result > 0 && libc::WIFSTOPPED(status) {
            assert_eq!(libc::WSTOPSIG(status), libc::SIGSTOP);
            return;
        }
        if result != 0 || std::time::Instant::now() >= deadline {
            let error = std::io::Error::last_os_error();
            let _ = child.kill();
            let _ = child.wait();
            panic!("stopped-child probe failed: waitpid={result}, status={status}, error={error}");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}
