use super::*;

#[test]
fn nested_mounts_below_ancestors_and_sealed_directories_are_supported() {
    const CHILD: &str = "ARCHON_NESTED_MOUNT_STAGE";
    const TEST: &str = "write_boundary::landlock::mounts::nested_tests::nested_mounts_below_ancestors_and_sealed_directories_are_supported";
    if let Some(stage) = std::env::var_os(CHILD) {
        let root = PathBuf::from(std::env::var_os("ARCHON_NESTED_MOUNT_ROOT").unwrap());
        if stage == "sealed" {
            assert_eq!(
                std::fs::write(root.join("sealed/nested/forged"), "x")
                    .unwrap_err()
                    .raw_os_error(),
                Some(libc::EROFS)
            );
            std::fs::write(root.join("ancestor/nested/allowed"), "x").unwrap();
            return;
        }
        // The next namespace copies locked child mounts. Ancestor binds must
        // recurse, and a directory seal must include its nested mount.
        let boundary = crate::write_boundary::landlock::LandlockSandbox::build_sealed(&[
            root.join("ancestor/config"),
            root.join("sealed"),
        ])
        .unwrap();
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", TEST, "--nocapture"])
            .env(CHILD, "sealed");
        boundary.install_std(&mut command);
        let output = command.output().unwrap();
        assert!(output.status.success(), "{output:?}");
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    for name in ["ancestor/nested", "sealed/nested"] {
        std::fs::create_dir_all(root.join(name)).unwrap();
    }
    std::fs::write(root.join("ancestor/config"), "original").unwrap();
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", TEST, "--nocapture"])
        .env(CHILD, "prepare")
        .env("ARCHON_NESTED_MOUNT_ROOT", &root);
    use std::os::unix::process::CommandExt;
    let uid = format!("{0} {0} 1\n", unsafe { libc::geteuid() }).into_bytes();
    let gid = format!("{0} {0} 1\n", unsafe { libc::getegid() }).into_bytes();
    let paths = ["ancestor/nested", "sealed/nested"]
        .map(|name| CString::new(root.join(name).as_os_str().as_bytes()).unwrap());
    // Only raw syscalls and prepared buffers after fork, before libtest starts.
    unsafe {
        command.pre_exec(move || {
            if libc::unshare(libc::CLONE_NEWUSER | libc::CLONE_NEWNS) != 0
                || write_map(c"/proc/self/setgroups", b"deny") != 0
                || write_map(c"/proc/self/uid_map", &uid) != 0
                || write_map(c"/proc/self/gid_map", &gid) != 0
                || libc::mount(
                    std::ptr::null(),
                    c"/".as_ptr(),
                    std::ptr::null(),
                    libc::MS_REC | libc::MS_PRIVATE,
                    std::ptr::null(),
                ) != 0
            {
                return Err(io::Error::last_os_error());
            }
            for path in &paths {
                if libc::mount(
                    c"tmpfs".as_ptr(),
                    path.as_ptr(),
                    c"tmpfs".as_ptr(),
                    0,
                    std::ptr::null(),
                ) != 0
                {
                    return Err(io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
    match command.output() {
        Ok(output) => assert!(output.status.success(), "{output:?}"),
        Err(error) => {
            assert!(
                std::env::var_os("CI").is_none(),
                "CI requires nested mounts: {error}"
            );
            eprintln!("nested mount test unavailable: {error}");
        }
    }
}
