use super::LandlockSandbox;

#[test]
fn bind_failure_keeps_execution_and_reports_errno() {
    let temp = tempfile::tempdir().unwrap();
    let protected = temp.path().join("protected");
    std::fs::create_dir(&protected).unwrap();
    let seals = match LandlockSandbox::build_sealed(std::slice::from_ref(&protected)) {
        Ok(seals) => seals,
        Err(reason) => {
            assert!(
                std::env::var_os("CI").is_none(),
                "CI requires private mounts: {reason}"
            );
            eprintln!("kernel test unavailable: {reason}");
            return;
        }
    };
    let active = super::probe().is_ok();
    assert!(
        active || std::env::var_os("CI").is_none(),
        "CI requires an active rule layer"
    );
    let rules =
        active.then(|| LandlockSandbox::build(std::slice::from_ref(&protected), &[], &[]).unwrap());
    let mut command = std::process::Command::new("/bin/sh");
    // The kernel rejects bind operations, while allowing the earlier namespace
    // setup and propagation operation. This happens after successful preflight.
    unsafe {
        std::os::unix::process::CommandExt::pre_exec(&mut command, deny_bind);
    }
    let report = crate::read_boundary::install_prepared_seals(&mut command, Ok(seals), active, &[]);
    if let Some(rules) = rules {
        rules.install_std(&mut command);
    }
    let output = command
        .args([
            "-c",
            "printf forbidden > \"$1/new\"; printf executed",
            "probe",
        ])
        .arg(&protected)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"executed");
    let report = report.to_string();
    assert!(report.contains("bind failed (errno 13)"), "{report}");
    assert!(report.contains("applied seals: []"), "{report}");
    assert!(!report.contains("pending"), "{report}");
    if active {
        assert!(
            !protected.join("new").exists(),
            "the independent rule must still apply"
        );
    }
}

fn deny_bind() -> std::io::Result<()> {
    let instruction = |code, jt, jf, k| libc::sock_filter { code, jt, jf, k };
    let instructions = [
        instruction(0x20, 0, 0, 0), // load syscall number
        instruction(0x15, 0, 3, libc::SYS_mount as u32),
        instruction(0x20, 0, 0, 40), // load low word of flags (argument 3)
        instruction(0x45, 0, 1, libc::MS_BIND as u32),
        instruction(0x06, 0, 0, 0x0005_0000 | libc::EACCES as u32),
        instruction(0x06, 0, 0, 0x7fff_0000),
    ];
    let program = libc::sock_fprog {
        len: instructions.len() as u16,
        filter: instructions.as_ptr().cast_mut(),
    };
    // SAFETY: fixed stack instructions retained throughout the calls.
    unsafe {
        if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0
            || libc::prctl(libc::PR_SET_SECCOMP, 2, &program) != 0
        {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}
