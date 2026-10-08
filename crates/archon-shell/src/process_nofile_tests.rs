use super::*;
use std::os::fd::AsRawFd;

#[test]
fn startup_limits_sweep_work_and_restores_child_limits() {
    for case in ["high", "low", "inherited", "unreadable"] {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "process_nofile::tests::startup_child",
                "--ignored",
                "--nocapture",
            ])
            .env("ARCHON_NOFILE_TEST", case)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{case}: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
#[ignore = "process limit isolation"]
fn startup_child() {
    let case = std::env::var("ARCHON_NOFILE_TEST").unwrap();
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    assert_eq!(
        unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) },
        0
    );
    limit.rlim_cur = if case == "low" { 2_048 } else { 16_384 };
    assert_eq!(unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limit) }, 0);
    let original = limit.rlim_cur;
    let file = std::fs::File::open("/dev/null").unwrap();
    let high = if case == "inherited" {
        let fd = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_DUPFD, 6_000) };
        assert!(fd >= 6_000);
        Some(fd)
    } else {
        None
    };
    if case == "unreadable" {
        // A sandbox that denies /dev/fd: startup goes on, nothing changes,
        // and the reason is kept for a warning.
        let missing = std::path::Path::new("/archon-no-such-fd-dir");
        unsafe { initialize_from(missing) };
        let reason = startup_degradation().expect("a degraded startup must say why");
        assert!(reason.contains("/archon-no-such-fd-dir"), "{reason}");
    } else {
        unsafe { initialize() };
        assert_eq!(startup_degradation(), None);
    }
    assert_eq!(
        unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) },
        0
    );
    let ceiling = crate::process_tree::descriptor_ceiling().unwrap();
    if case == "unreadable" {
        assert_eq!(
            limit.rlim_cur, original,
            "a degraded startup changes nothing"
        );
        // No descriptor can exist above the process's table size, which an
        // OS may hold below the soft limit (macOS: kern.maxfilesperproc).
        #[cfg(target_vendor = "apple")]
        let reachable = original.min(unsafe { libc::getdtablesize() } as u64);
        #[cfg(not(target_vendor = "apple"))]
        let reachable = original;
        assert!(
            ceiling as u64 >= reachable,
            "the fallback sweep must cover every reachable descriptor: {ceiling} < {reachable}"
        );
    } else {
        assert_eq!(limit.rlim_cur, original.min(STARTUP_SOFT_LIMIT));
        assert!(
            ceiling as u64 <= STARTUP_SOFT_LIMIT.max(6_001),
            "startup must bound the sweep: {ceiling}"
        );
    }
    let script = match high {
        Some(fd) => format!("test ! -e /dev/fd/{fd} || exit 33; ulimit -n"),
        None => "ulimit -n".into(),
    };
    let out = crate::spawn::command("/bin/sh")
        .args(["-c", &script])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout)
            .trim()
            .parse::<u64>()
            .unwrap(),
        original
    );
    if let Some(fd) = high {
        unsafe { libc::close(fd) };
    }
}
